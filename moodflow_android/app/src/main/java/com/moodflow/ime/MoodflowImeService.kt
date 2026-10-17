package com.moodflow.ime

import android.content.ClipboardManager
import android.content.Intent
import android.inputmethodservice.InputMethodService
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import android.view.KeyEvent
import android.view.View
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.ExtractedTextRequest
import android.view.inputmethod.InputConnection
import android.widget.Toast
import com.moodflow.ime.core.DataInstaller
import com.moodflow.ime.core.MoodflowCore
import com.moodflow.ime.core.NativeBuffer
import com.moodflow.ime.core.NativeChord
import com.moodflow.ime.core.NativeClipboard
import com.moodflow.ime.core.T9Native
import com.moodflow.ime.jni.MoodflowNative
import com.moodflow.ime.jni.NativeEventSink
import com.moodflow.ime.logic.BufferSink
import com.moodflow.ime.logic.CandidateWindow
import com.moodflow.ime.logic.ChordResult
import com.moodflow.ime.logic.ClipEntry
import com.moodflow.ime.logic.ClipboardGate
import com.moodflow.ime.logic.EditOp
import com.moodflow.ime.logic.FieldChange
import com.moodflow.ime.logic.FieldKey
import com.moodflow.ime.logic.FieldPolicy
import com.moodflow.ime.logic.FieldTracker
import com.moodflow.ime.logic.KeyOutcome
import com.moodflow.ime.logic.Mods
import com.moodflow.ime.logic.OpRouter
import com.moodflow.ime.logic.PhysicalKeys
import com.moodflow.ime.logic.StrokeTable
import com.moodflow.ime.logic.T9Input
import com.moodflow.ime.logic.TextSink
import com.moodflow.ime.logic.Vk
import com.moodflow.ime.ui.BufferAction
import com.moodflow.ime.ui.KeyboardActions
import com.moodflow.ime.ui.KeyboardLayout
import com.moodflow.ime.ui.KeyboardState
import com.moodflow.ime.ui.KeyboardView
import java.io.File
import java.lang.ref.WeakReference

/**
 * Moodflow 输入法服务。
 *
 * 输入逻辑全部在核心（`moodflow-mobile` → headless 协调器）；这里只做三件事：
 * 1. 把按键送进核心，把核心返回的编辑指令写进 `InputConnection`（或 Buffer）；
 * 2. 维护输入框生命周期：字段隐私判据、Buffer 的绑定与失效、剪贴板只在前台采集；
 * 3. 画键盘。
 *
 * 线程：除核心构造在后台线程外，全部在主线程。核心的推送回调（候选、状态）来自核心泵线程，
 * 经 [CoreEvents] 切回主线程。
 *
 * 日志：只记生命周期事件，**不记任何用户输入、候选或剪贴板内容**（AGENTS.md 日志规范）。
 */
class MoodflowImeService : InputMethodService(), KeyboardActions {
    private val main = Handler(Looper.getMainLooper())
    private var destroyed = false

    private var core: MoodflowCore? = null
    private var coreFailed = false
    private var strokes = StrokeTable.EMPTY
    private var storeAvailable = true
    private var chinese = true
    private var statusLabel = ""

    private val tracker = FieldTracker()
    private var policy = FieldPolicy.NONE

    private var buffer: NativeBuffer? = null
    private var bufferMode = false
    private var clipboard: NativeClipboard? = null
    private var clipboardOpen = false
    private var clipListening = false
    private var chord: NativeChord? = null
    private val t9 = T9Input()

    private var layout = KeyboardLayout.QWERTY
    private var letterLayout = KeyboardLayout.QWERTY
    private var shifted = false
    private var view: KeyboardView? = null
    private var keyboardVisible = false

    // 候选态：推送帧给编码串，列表按窗口从核心拉
    private var preedit = ""
    private var composing = false
    private var candidates = CandidateWindow.EMPTY
    private var pinyinRow: List<String> = emptyList()

    /** 物理键盘 keydown 被核心吃掉的键，对应的 keyup 也吞掉，不让应用收到半个按键。 */
    private val swallowedKeyUps = HashSet<Int>()

    private val clipListener = ClipboardManager.OnPrimaryClipChangedListener { onClipChanged() }
    private val chordTick = Runnable { chord?.tick(SystemClock.uptimeMillis())?.let { handleChord(it) } }

    private val bufferSink = object : BufferSink {
        override fun pushPhrase(text: String): Boolean = buffer?.pushPhrase(text) == 0
    }

    // ── 生命周期 ────────────────────────────────────────────────

    override fun onCreate() {
        super.onCreate()
        letterLayout = if (Prefs.of(this).getBoolean(Prefs.T9_DEFAULT, false)) {
            KeyboardLayout.T9
        } else {
            KeyboardLayout.QWERTY
        }
        buffer = NativeBuffer.create()
        clipboard = NativeClipboard.open(File(DataInstaller.userRoot(this), "clipboard.json"), nowSecs())
        chord = loadChord()
        MoodflowNative.loadError?.let { Log.e(TAG, "原生库加载失败，键盘退化为直接上屏", it) }
        startCore()
    }

    private fun startCore() {
        val events = CoreEvents(WeakReference(this), main)
        val app = applicationContext
        Thread({
            val c = try {
                val data = DataInstaller.install(app)
                MoodflowCore.create(data, DataInstaller.userRoot(app), FALLBACK_SCHEMA, events)
            } catch (e: Exception) {
                Log.e(TAG, "核心启动失败", e)
                null
            }
            main.post { onCoreReady(c) }
        }, "moodflow-core-init").start()
    }

    private fun onCoreReady(c: MoodflowCore?) {
        if (destroyed) {
            c?.close()
            return
        }
        if (c == null) {
            coreFailed = true
            render()
            return
        }
        core = c
        strokes = c.charStrokes()
        storeAvailable = c.userStoreAvailable()
        if (tracker.current != null) c.focusGained(policy.isPrivate)
        if (letterLayout == KeyboardLayout.T9) ensurePinyinSchema()
        Log.i(TAG, "核心就绪")
        render()
    }

    override fun onDestroy() {
        destroyed = true
        stopClipListening()
        main.removeCallbacksAndMessages(null)
        core?.close()
        core = null
        buffer?.close()
        buffer = null
        clipboard?.close()
        clipboard = null
        chord?.close()
        chord = null
        super.onDestroy()
    }

    override fun onCreateInputView(): View {
        val v = KeyboardView(this, this)
        view = v
        render()
        return v
    }

    /** 横屏也不进全屏抽取界面：候选与 Buffer 都画在键盘里。 */
    override fun onEvaluateFullscreenMode(): Boolean = false

    override fun onStartInput(attribute: EditorInfo, restarting: Boolean) {
        super.onStartInput(attribute, restarting)
        policy = FieldPolicy.of(attribute.inputType, attribute.imeOptions)
        val change = tracker.onStartInput(
            FieldKey(attribute.packageName.orEmpty(), attribute.fieldId, attribute.inputType),
            restarting,
        )
        // 旧组字绝不带进新上下文（XSIME 吸收计划 §3.7）：核心与本地状态一起清
        core?.clearComposition()
        resetLocalComposition()
        chord?.reset()
        swallowedKeyUps.clear()

        // Buffer：换字段丢弃，同字段重启只暂停（buffer.md §3 Android 一列）；私密字段一律丢弃
        val target = tracker.current
        buffer?.let { b ->
            if (!policy.allowBuffer || change == FieldChange.NEW_FIELD || target == null) {
                b.discard()
            } else {
                b.focusChanged(target)
            }
        }
        if (!policy.allowBuffer || change == FieldChange.NEW_FIELD) bufferMode = false
        if (change == FieldChange.NEW_FIELD) clipboardOpen = false

        core?.focusGained(policy.isPrivate)
        shifted = false
        layout = when {
            policy.prefersNumberPad -> KeyboardLayout.NUMBER
            policy.isPrivate -> KeyboardLayout.QWERTY
            else -> letterLayout
        }
        render()
    }

    override fun onStartInputView(info: EditorInfo, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        keyboardVisible = true
        startClipListening()
        render()
    }

    override fun onFinishInputView(finishingInput: Boolean) {
        keyboardHidden()
        super.onFinishInputView(finishingInput)
    }

    override fun onWindowHidden() {
        keyboardHidden()
        super.onWindowHidden()
    }

    /** 隐藏键盘：停止剪贴板采集，Buffer 清空（Android 上隐藏即撤销投递授权并丢弃内容）。 */
    private fun keyboardHidden() {
        keyboardVisible = false
        stopClipListening()
        clipboardOpen = false
        buffer?.discard()
        bufferMode = false
        if (composing || !t9.isEmpty) {
            core?.clearComposition()
            clearHostComposition()
            resetLocalComposition()
        }
    }

    override fun onFinishInput() {
        // 旧输入框里的组字删掉而不是提交：拼音串不是用户要的内容，也不能留给下一个输入框
        clearHostComposition()
        tracker.onFinishInput()
        core?.focusLost()
        buffer?.discard()
        bufferMode = false
        chord?.reset()
        resetLocalComposition()
        policy = FieldPolicy.NONE
        super.onFinishInput()
    }

    // ── 核心推送（已切回主线程）────────────────────────────────

    private fun onCandidateFrame(preeditText: String) {
        preedit = preeditText
        composing = true
        candidates = core?.candidates(0, CANDIDATE_WINDOW) ?: CandidateWindow.EMPTY
        render()
    }

    private fun onCandidatesHidden() {
        // 九宫格状态不在这里清：replaceComposition 内部的 Esc 也会推一次隐藏帧
        preedit = ""
        composing = false
        candidates = CandidateWindow.EMPTY
        render()
    }

    private fun onStatus(chineseMode: Boolean, iconLabel: String) {
        chinese = chineseMode
        statusLabel = iconLabel
        render()
    }

    private fun showToast(text: String) {
        Toast.makeText(this, text, Toast.LENGTH_SHORT).show()
    }

    // ── 键盘动作 ────────────────────────────────────────────────

    override fun currentGeneration(): Long = tracker.generation

    override fun isCurrent(generation: Long): Boolean = tracker.isCurrent(generation)

    /** 本字段该不该走引擎：私密 / 非文本字段、核心未就绪时为 null，按键直接上屏。 */
    private fun engine(): MoodflowCore? = if (policy.useEngine) core else null

    override fun onChar(ch: Char) {
        val c = engine()
        val stroke = strokes[ch]
        if (c == null || stroke == null) {
            commitRaw(ch.toString())
        } else {
            val o = c.keyDown(stroke.vk, if (stroke.shift) Mods.SHIFT else Mods.NONE)
            if (o.consumed) applyOutcome(o) else commitRaw(ch.toString())
        }
        if (shifted) {
            shifted = false
            render()
        }
    }

    override fun onBackspace() {
        val c = engine()
        if (c != null && layout == KeyboardLayout.T9 && !t9.isEmpty) {
            t9.backspace()
            refreshT9()
            return
        }
        if (c != null) {
            val o = c.keyDown(Vk.BACK, Mods.NONE)
            if (o.consumed) {
                applyOutcome(o)
                return
            }
        }
        val b = buffer
        if (bufferMode && b != null && b.blocks().isNotEmpty()) {
            b.backspace()
            render()
            return
        }
        sendDownUpKeyEvents(KeyEvent.KEYCODE_DEL)
    }

    override fun onSpace() {
        val c = engine()
        if (c != null) {
            val o = c.keyDown(Vk.SPACE, Mods.NONE)
            if (o.consumed) {
                applyOutcome(o)
                return
            }
        }
        commitRaw(" ")
    }

    override fun onEnter() {
        val b = buffer
        if (bufferMode && b != null) {
            // 有组字时只确认组字，同一次按键不顺带发送（buffer.md 契约 #7）
            when (b.sendKeyAction(composing)) {
                NativeBuffer.SendKey.CONFIRM_COMPOSITION -> {
                    engine()?.keyDown(Vk.RETURN, Mods.NONE)?.let { applyOutcome(it) }
                    return
                }
                NativeBuffer.SendKey.SEND_NEXT -> {
                    deliverBuffer(all = false)
                    return
                }
                NativeBuffer.SendKey.PASS_THROUGH -> Unit
            }
        }
        val c = engine()
        if (c != null) {
            val o = c.keyDown(Vk.RETURN, Mods.NONE)
            if (o.consumed) {
                applyOutcome(o)
                return
            }
        }
        if (!sendDefaultEditorAction(true)) sendDownUpKeyEvents(KeyEvent.KEYCODE_ENTER)
    }

    override fun onShift() {
        shifted = !shifted
        render()
    }

    override fun onT9Digit(d: Char) {
        if (engine() == null) {
            commitRaw(d.toString())
            return
        }
        t9.pressDigit(d)
        refreshT9()
    }

    override fun onT9Separator() {
        if (engine() == null) {
            commitRaw("1")
            return
        }
        t9.pressSeparator()
        refreshT9()
    }

    override fun onPinSyllable(syllable: String) {
        T9Native.pin(t9.text, syllable)?.let { t9.replaceWith(it) }
        refreshT9()
    }

    /**
     * 按当前九宫格串重算：取第一种切分整体替换核心里的组字。
     *
     * TODO(P3): 多种切分之间的取舍目前只看段数与字典序（`moodflow_engine::pinyin::t9`），
     * 应改为按引擎候选质量排序；部分上屏后剩余数字串与核心组字的同步也未处理（上屏即清空九宫格串）。
     */
    private fun refreshT9() {
        val c = engine() ?: return
        if (t9.isEmpty) {
            pinyinRow = emptyList()
            applyOutcome(c.clearComposition())
            render()
            return
        }
        pinyinRow = T9Native.leading(t9.text).take(T9_PINYIN_ROW)
        val query = T9Native.segmentations(t9.text, T9_SEGMENTATIONS).firstOrNull()
        if (query == null) {
            // 没有合法切分：清掉组字，编码栏直接显示数字串
            applyOutcome(c.clearComposition())
            preedit = t9.text
        } else {
            applyOutcome(c.replaceComposition(query))
        }
        render()
    }

    override fun onCandidate(index: Int) {
        val c = core ?: return
        applyOutcome(c.selectCandidate(index))
    }

    override fun onToggleChinese() {
        val c = core ?: return
        if (chinese) {
            c.runCommand(MoodflowCore.Command.ENGLISH)
        } else {
            val idx = c.schemas().indexOfFirst { it.active }.coerceAtLeast(0)
            c.runCommand(MoodflowCore.Command.SELECT_SCHEMA, idx)
        }
    }

    override fun onSwitchLayout(layout: KeyboardLayout) {
        this.layout = layout
        if (layout == KeyboardLayout.QWERTY || layout == KeyboardLayout.T9) {
            letterLayout = layout
            Prefs.of(this).edit().putBoolean(Prefs.T9_DEFAULT, layout == KeyboardLayout.T9).apply()
        }
        // 选九宫格时自动用拼音（XSIME 吸收计划 §2.6）
        if (layout == KeyboardLayout.T9) ensurePinyinSchema()
        if (layout != KeyboardLayout.T9 && !t9.isEmpty) {
            t9.clear()
            pinyinRow = emptyList()
            core?.clearComposition()?.let { applyOutcome(it) }
        }
        shifted = false
        render()
    }

    private fun ensurePinyinSchema() {
        val c = core ?: return
        val list = c.schemas()
        val idx = list.indexOfFirst { it.id == FALLBACK_SCHEMA }
        if (idx >= 0 && (!list[idx].active || !chinese)) {
            c.runCommand(MoodflowCore.Command.SELECT_SCHEMA, idx)
        }
    }

    // ── Buffer ──────────────────────────────────────────────────

    override fun onToggleBuffer() {
        val b = buffer
        if (b == null) {
            showToast("缓冲不可用（原生库未加载）")
            return
        }
        if (!policy.allowBuffer) return
        bufferMode = !bufferMode
        if (bufferMode) {
            // 打开 Buffer 栏即绑定当前字段（buffer.md §3）
            tracker.current?.let { b.bind(it, policy.isPrivate) }
        } else {
            // 正文是临时数据（buffer.md 契约 #12），关掉即丢弃
            b.discard()
        }
        render()
    }

    override fun onBufferAction(action: BufferAction) {
        val b = buffer ?: return
        val t = tracker.current
        when (action) {
            BufferAction.BIND -> if (t == null || !b.bind(t, policy.isPrivate)) showToast("此输入框不能使用缓冲")
            BufferAction.SEND_NEXT -> deliverBuffer(all = false)
            BufferAction.SEND_ALL -> deliverBuffer(all = true)
            BufferAction.DELETE_LAST -> b.deleteLast()
            BufferAction.CLEAR -> b.clear()
            BufferAction.RELEASE_HOLD -> b.releaseHold()
        }
        render()
    }

    /** 两段式投递：取出 → 写入前复核授权 → 按 `commitText` 结果报告（buffer.md §2）。 */
    private fun deliverBuffer(all: Boolean) {
        val b = buffer ?: return
        val t = tracker.current ?: return
        val text = b.begin(t, all)
        if (text == null) {
            showToast(
                when (b.state()) {
                    NativeBuffer.State.UNBOUND, NativeBuffer.State.SUSPENDED -> "请先绑定当前输入框"
                    NativeBuffer.State.HELD -> "上次投递结果不明，确认后再继续"
                    NativeBuffer.State.READY -> "缓冲为空"
                },
            )
            render()
            return
        }
        val ic = currentInputConnection
        if (ic == null) {
            // 连接已断开、无法确认是否写入：保留并暂停，不自动重发
            b.finish(NativeBuffer.WriteResult.UNKNOWN)
            render()
            return
        }
        if (!b.isAuthorized(t)) {
            render()
            return
        }
        val ok = ic.commitText(text, 1)
        b.finish(if (ok) NativeBuffer.WriteResult.ACCEPTED else NativeBuffer.WriteResult.REJECTED)
        render()
    }

    // ── 剪贴板 ──────────────────────────────────────────────────

    override fun onToggleClipboard() {
        clipboardOpen = !clipboardOpen
        render()
    }

    override fun onClipboardPick(entry: ClipEntry) {
        clipboardOpen = false
        val b = buffer
        if (bufferMode && b != null) {
            b.pushPhrase(entry.text)
        } else {
            core?.clearComposition()
            resetLocalComposition()
            currentInputConnection?.commitText(entry.text, 1)
        }
        render()
    }

    override fun onClipboardPin(entry: ClipEntry) {
        clipboard?.setPinned(entry.id, !entry.pinned)
        render()
    }

    override fun onClipboardClear() {
        clipboard?.clearUnpinned()
        render()
    }

    private fun startClipListening() {
        if (clipListening || !Prefs.of(this).getBoolean(Prefs.CLIPBOARD_HISTORY, true)) return
        val cm = getSystemService(ClipboardManager::class.java) ?: return
        cm.addPrimaryClipChangedListener(clipListener)
        clipListening = true
    }

    private fun stopClipListening() {
        if (!clipListening) return
        getSystemService(ClipboardManager::class.java)?.removePrimaryClipChangedListener(clipListener)
        clipListening = false
    }

    /**
     * 剪贴板变化。Android 10+ 只有前台的默认输入法读得到剪贴板，所以只在键盘可见时采集
     * （XSIME 吸收计划 §2.4）；私密字段与平台标记敏感的内容不收录。
     */
    private fun onClipChanged() {
        if (!ClipboardGate.shouldCapture(keyboardVisible, policy)) return
        val store = clipboard ?: return
        val cm = getSystemService(ClipboardManager::class.java) ?: return
        val clip = cm.primaryClip ?: return
        if (clip.itemCount == 0) return
        val platformSensitive = clip.description?.extras?.getBoolean(EXTRA_IS_SENSITIVE, false) == true
        val text = clip.getItemAt(0).coerceToText(this)?.toString() ?: return
        store.record(text, ClipboardGate.sensitive(platformSensitive, policy), nowSecs())
        if (clipboardOpen) render()
    }

    override fun onOpenSettings() {
        startActivity(Intent(this, SettingsActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }

    // ── 物理键盘 ────────────────────────────────────────────────

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val c = engine()
        if (c == null || tracker.current == null) return super.onKeyDown(keyCode, event)
        val mods = PhysicalKeys.toMods(event.metaState)
        val ch = chord
        val name = PhysicalKeys.chordKeyName(keyCode)
        if (ch != null && name != null && mods and (Mods.CTRL or Mods.ALT) == 0 && ch.isChordKey(name)) {
            // 并击：按下只记账，全部松开时结算（moodflow-chord）
            if (event.repeatCount == 0) handleChord(ch.keyDown(name, event.eventTime))
            return true
        }
        val vk = PhysicalKeys.toVk(keyCode) ?: return super.onKeyDown(keyCode, event)
        val o = c.keyDown(vk, mods)
        if (!o.consumed) return super.onKeyDown(keyCode, event)
        swallowedKeyUps += keyCode
        applyOutcome(o)
        return true
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        val ch = chord
        val name = PhysicalKeys.chordKeyName(keyCode)
        if (engine() != null && ch != null && name != null && ch.isChordKey(name)) {
            handleChord(ch.keyUp(name, event.eventTime))
            // 先敲一只手时要等另一只手：超时后单独结算
            main.removeCallbacks(chordTick)
            main.postDelayed(chordTick, CHORD_TICK_SHORT_MS)
            main.postDelayed(chordTick, CHORD_TICK_LONG_MS)
            return true
        }
        if (swallowedKeyUps.remove(keyCode)) return true
        return super.onKeyUp(keyCode, event)
    }

    private fun handleChord(r: ChordResult?) {
        when (r) {
            is ChordResult.Emit -> engine()?.feedText(r.pinyin)?.let { applyOutcome(it) }
            ChordResult.NoMatch -> Unit
            null -> Unit
        }
    }

    private fun loadChord(): NativeChord? {
        val f = File(filesDir, Prefs.CHORD_FILE)
        if (!f.isFile) return null
        val c = NativeChord.fromJson(f.readText())
        if (c == null) Log.w(TAG, "并击方案无法应用，已忽略")
        return c
    }

    // ── 输出 ────────────────────────────────────────────────────

    /** 把核心的编辑指令写进宿主或 Buffer。 */
    private fun applyOutcome(o: KeyOutcome) {
        val ic = currentInputConnection
        val sink: TextSink = if (ic != null) IcSink(ic) else NullSink
        val rejected = OpRouter.apply(o.ops, sink, if (bufferMode) bufferSink else null)
        if (rejected.isNotEmpty()) showToast("缓冲已满，这次的内容没有收下")
        if (!t9.isEmpty && o.ops.any { it is EditOp.Commit }) {
            t9.clear()
            pinyinRow = emptyList()
        }
        if (bufferMode || o.modeChanged) render()
    }

    /** 不经引擎直接上屏（私密字段、引擎未就绪、核心不收的键）。Buffer 开着时进 Buffer。 */
    private fun commitRaw(text: String) {
        val b = buffer
        if (bufferMode && b != null) {
            if (text.all { it.code in 0x21..0x7E }) {
                text.forEach { b.pushLatin(it.code) }
            } else {
                b.pushPhrase(text)
            }
            render()
            return
        }
        currentInputConnection?.commitText(text, 1)
    }

    private fun clearHostComposition() {
        currentInputConnection?.let {
            it.setComposingText("", 1)
            it.finishComposingText()
        }
    }

    private fun resetLocalComposition() {
        preedit = ""
        composing = false
        candidates = CandidateWindow.EMPTY
        t9.clear()
        pinyinRow = emptyList()
    }

    private fun render() {
        val v = view ?: return
        val b = buffer
        val bState = if (bufferMode && b != null) b.state() else NativeBuffer.State.UNBOUND
        v.render(
            KeyboardState(
                layout = layout,
                shifted = shifted,
                chinese = chinese && policy.useEngine,
                statusLabel = if (policy.useEngine) statusLabel else "直",
                preedit = preedit,
                notice = notice(),
                candidates = candidates.items,
                candidateOffset = candidates.offset,
                selected = candidates.selected,
                pinyinRow = pinyinRow,
                bufferAllowed = policy.allowBuffer && b != null,
                bufferMode = bufferMode,
                bufferBlocks = if (bufferMode) b?.blocks().orEmpty() else emptyList(),
                bufferStateLabel = when (bState) {
                    NativeBuffer.State.UNBOUND -> "未绑定"
                    NativeBuffer.State.READY -> "已绑定"
                    NativeBuffer.State.SUSPENDED -> "已暂停"
                    NativeBuffer.State.HELD -> "待确认"
                },
                bufferNeedsBind = bState == NativeBuffer.State.UNBOUND || bState == NativeBuffer.State.SUSPENDED,
                bufferHeld = bState == NativeBuffer.State.HELD,
                clipboardOpen = clipboardOpen,
                clipEntries = if (clipboardOpen) clipboard?.entries().orEmpty() else emptyList(),
            ),
        )
    }

    private fun notice(): String = when {
        tracker.current != null && policy.isPrivate -> "私密输入框：不经引擎、不学习、不记录剪贴板"
        coreFailed -> "输入引擎不可用，按键直接上屏"
        core == null -> "输入引擎加载中…"
        !storeAvailable -> "⚠ 用户词库不可写：学到的词不会保存"
        else -> ""
    }

    private fun nowSecs(): Long = System.currentTimeMillis() / 1000

    /** `InputConnection` → [TextSink]。 */
    private class IcSink(private val ic: InputConnection) : TextSink {
        override fun setComposing(text: String, caret: Int) {
            if (text.isEmpty()) {
                ic.setComposingText("", 1)
                ic.finishComposingText()
            } else {
                ic.setComposingText(text, 1)
            }
        }

        override fun commit(text: String) {
            ic.commitText(text, 1)
        }

        override fun deleteBefore(codePoints: Int) {
            ic.deleteSurroundingTextInCodePoints(codePoints, 0)
        }

        override fun replaceBefore(codePoints: Int, text: String) {
            // 同一个批量编辑：宿主撤销栈记成一步
            ic.beginBatchEdit()
            ic.deleteSurroundingTextInCodePoints(codePoints, 0)
            ic.commitText(text, 1)
            ic.endBatchEdit()
        }

        override fun moveCursor(delta: Int) {
            val et = ic.getExtractedText(ExtractedTextRequest(), 0) ?: return
            val pos = (et.startOffset + et.selectionEnd + delta).coerceAtLeast(0)
            ic.setSelection(pos, pos)
        }
    }

    private object NullSink : TextSink {
        override fun setComposing(text: String, caret: Int) = Unit
        override fun commit(text: String) = Unit
        override fun deleteBefore(codePoints: Int) = Unit
        override fun replaceBefore(codePoints: Int, text: String) = Unit
        override fun moveCursor(delta: Int) = Unit
    }

    /** 核心推送 → 主线程。只持弱引用：核心的回调引用活得可能比服务久。 */
    private class CoreEvents(
        private val service: WeakReference<MoodflowImeService>,
        private val main: Handler,
    ) : NativeEventSink {
        override fun onCandidates(
            preedit: String,
            caret: Int,
            modeLabel: String,
            items: Array<String>,
            selected: Int,
            page: Int,
            totalPages: Int,
        ) {
            main.post { service.get()?.onCandidateFrame(preedit) }
        }

        override fun onHideCandidates() {
            main.post { service.get()?.onCandidatesHidden() }
        }

        override fun onStatus(
            chineseMode: Boolean,
            iconLabel: String,
            fullWidth: Boolean,
            chinesePunct: Boolean,
            s2tEnabled: Boolean,
            s2tShown: Boolean,
        ) {
            main.post { service.get()?.onStatus(chineseMode, iconLabel) }
        }

        override fun onToast(text: String) {
            main.post { service.get()?.showToast(text) }
        }
    }

    private companion object {
        const val TAG = "Moodflow"
        const val FALLBACK_SCHEMA = "pinyin"
        const val CANDIDATE_WINDOW = 60
        const val T9_SEGMENTATIONS = 8
        const val T9_PINYIN_ROW = 12
        const val CHORD_TICK_SHORT_MS = 400L
        const val CHORD_TICK_LONG_MS = 2100L

        /** `ClipDescription.EXTRA_IS_SENSITIVE`（API 33 起的常量，按字面量用以兼容低版本）。 */
        const val EXTRA_IS_SENSITIVE = "android.content.extra.IS_SENSITIVE"
    }
}
