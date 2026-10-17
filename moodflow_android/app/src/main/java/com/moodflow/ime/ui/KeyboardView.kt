package com.moodflow.ime.ui

import android.annotation.SuppressLint
import android.content.Context
import android.content.res.Configuration
import android.graphics.Color
import android.graphics.Typeface
import android.text.TextUtils
import android.util.TypedValue
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.widget.Button
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import com.moodflow.ime.logic.ClipEntry
import com.moodflow.ime.logic.T9Input

enum class KeyboardLayout { QWERTY, SYMBOLS, T9, NUMBER }

enum class BufferAction { BIND, SEND_NEXT, SEND_ALL, DELETE_LAST, CLEAR, RELEASE_HOLD }

/** 键盘一次渲染所需的全部数据。由服务组装，视图只画不想。 */
data class KeyboardState(
    val layout: KeyboardLayout,
    val shifted: Boolean,
    val chinese: Boolean,
    val statusLabel: String,
    val preedit: String,
    /** 编码栏为空时显示的提示（引擎加载中、私密字段、词库不可写…） */
    val notice: String,
    val candidates: List<String>,
    val candidateOffset: Int,
    /** 高亮候选的绝对下标 */
    val selected: Int,
    /** 九宫格「选拼音」行 */
    val pinyinRow: List<String>,
    val bufferAllowed: Boolean,
    val bufferMode: Boolean,
    val bufferBlocks: List<String>,
    val bufferStateLabel: String,
    val bufferNeedsBind: Boolean,
    val bufferHeld: Boolean,
    val clipboardOpen: Boolean,
    val clipEntries: List<ClipEntry>,
)

/** 视图 → 服务的全部动作。 */
interface KeyboardActions {
    /** 当前输入框的「代」；按下时记下，抬起处理时比对，换输入框后迟到的点击作废。 */
    fun currentGeneration(): Long
    fun isCurrent(generation: Long): Boolean
    fun onChar(ch: Char)
    fun onBackspace()
    fun onEnter()
    fun onSpace()
    fun onShift()
    fun onT9Digit(d: Char)
    fun onT9Separator()
    fun onPinSyllable(syllable: String)
    fun onCandidate(index: Int)
    fun onToggleChinese()
    fun onSwitchLayout(layout: KeyboardLayout)
    fun onToggleBuffer()
    fun onBufferAction(action: BufferAction)
    fun onToggleClipboard()
    fun onClipboardPick(entry: ClipEntry)
    fun onClipboardPin(entry: ClipEntry)
    fun onClipboardClear()
    fun onOpenSettings()
}

/**
 * 键盘视图：工具栏、Buffer 栏、编码栏、九宫格拼音行、候选栏、剪贴板面板、键区。
 *
 * 纯代码搭建（不用布局 XML）：键区随布局 / 大小写整块重建，其余行按需刷新。
 */
@SuppressLint("ViewConstructor")
class KeyboardView(context: Context, private val actions: KeyboardActions) : LinearLayout(context) {
    private val dark = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) ==
        Configuration.UI_MODE_NIGHT_YES
    private val bg = if (dark) Color.rgb(0x20, 0x22, 0x26) else Color.rgb(0xEC, 0xEE, 0xF1)
    private val fg = if (dark) Color.rgb(0xE8, 0xE8, 0xE8) else Color.rgb(0x20, 0x20, 0x20)
    private val accent = Color.rgb(0x3A, 0x7B, 0xD5)
    private val muted = if (dark) Color.rgb(0x9A, 0x9A, 0x9A) else Color.rgb(0x70, 0x70, 0x70)

    private val toolbar = LinearLayout(context)
    private val bufferBar = LinearLayout(context)
    private val bufferText = TextView(context)
    private val bufferButtons = LinearLayout(context)
    private val preeditView = TextView(context)
    private val pinyinScroll = HorizontalScrollView(context)
    private val pinyinRow = LinearLayout(context)
    private val candScroll = HorizontalScrollView(context)
    private val candRow = LinearLayout(context)
    private val clipScroll = ScrollView(context)
    private val clipList = LinearLayout(context)
    private val keysArea = LinearLayout(context)

    private var builtLayout: KeyboardLayout? = null
    private var builtShift = false
    private var lastCandidates: List<String>? = null
    private var lastCandidateKey = Pair(-1, -1)

    init {
        orientation = VERTICAL
        setBackgroundColor(bg)

        toolbar.orientation = HORIZONTAL
        addView(toolbar, LayoutParams(LayoutParams.MATCH_PARENT, dp(TOOLBAR_H)))

        bufferBar.orientation = VERTICAL
        bufferText.setTextColor(fg)
        bufferText.textSize = 15f
        bufferText.maxLines = 3
        bufferText.ellipsize = TextUtils.TruncateAt.START
        bufferText.setPadding(dp(8), dp(4), dp(8), dp(2))
        bufferBar.addView(bufferText, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
        bufferButtons.orientation = HORIZONTAL
        bufferBar.addView(bufferButtons, LayoutParams(LayoutParams.MATCH_PARENT, dp(TOOLBAR_H)))
        bufferBar.visibility = GONE
        addView(bufferBar, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))

        preeditView.setTextColor(muted)
        preeditView.textSize = 14f
        preeditView.setSingleLine()
        preeditView.setPadding(dp(10), dp(2), dp(10), 0)
        addView(preeditView, LayoutParams(LayoutParams.MATCH_PARENT, dp(PREEDIT_H)))

        pinyinRow.orientation = HORIZONTAL
        pinyinScroll.isHorizontalScrollBarEnabled = false
        pinyinScroll.addView(pinyinRow)
        pinyinScroll.visibility = GONE
        addView(pinyinScroll, LayoutParams(LayoutParams.MATCH_PARENT, dp(CAND_H)))

        candRow.orientation = HORIZONTAL
        candScroll.isHorizontalScrollBarEnabled = false
        candScroll.addView(candRow)
        addView(candScroll, LayoutParams(LayoutParams.MATCH_PARENT, dp(CAND_H)))

        clipList.orientation = VERTICAL
        clipScroll.addView(clipList)
        clipScroll.visibility = GONE
        addView(clipScroll, LayoutParams(LayoutParams.MATCH_PARENT, dp(KEY_H * 4 + 24)))

        keysArea.orientation = VERTICAL
        keysArea.setPadding(dp(2), dp(2), dp(2), dp(4))
        addView(keysArea, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
    }

    fun render(s: KeyboardState) {
        renderToolbar(s)
        renderBuffer(s)
        preeditView.text = s.preedit.ifEmpty { s.notice }
        preeditView.setTextColor(if (s.preedit.isNotEmpty()) fg else muted)
        renderPinyinRow(s.pinyinRow)
        renderCandidates(s)
        if (s.clipboardOpen) {
            clipScroll.visibility = VISIBLE
            keysArea.visibility = GONE
            renderClipboard(s.clipEntries)
        } else {
            clipScroll.visibility = GONE
            keysArea.visibility = VISIBLE
        }
        if (builtLayout != s.layout || builtShift != s.shifted) {
            buildKeys(s.layout, s.shifted)
            builtLayout = s.layout
            builtShift = s.shifted
        }
    }

    // ── 工具栏 ──

    private fun renderToolbar(s: KeyboardState) {
        toolbar.removeAllViews()
        toolbar.addView(toolButton(s.statusLabel.ifEmpty { if (s.chinese) "中" else "英" }, s.chinese) {
            actions.onToggleChinese()
        })
        val t9 = s.layout == KeyboardLayout.T9
        toolbar.addView(toolButton(if (t9) "全键盘" else "九宫格", false) {
            actions.onSwitchLayout(if (t9) KeyboardLayout.QWERTY else KeyboardLayout.T9)
        })
        if (s.bufferAllowed) {
            toolbar.addView(toolButton("缓冲", s.bufferMode) { actions.onToggleBuffer() })
        }
        toolbar.addView(toolButton("剪贴板", s.clipboardOpen) { actions.onToggleClipboard() })
        toolbar.addView(toolButton("设置", false) { actions.onOpenSettings() })
    }

    private fun toolButton(label: String, active: Boolean, onPress: () -> Unit): View {
        val b = baseButton(label, 14f, onPress)
        b.setTextColor(if (active) accent else fg)
        b.layoutParams = LayoutParams(0, LayoutParams.MATCH_PARENT, 1f)
        return b
    }

    // ── Buffer 栏 ──

    private fun renderBuffer(s: KeyboardState) {
        if (!s.bufferMode) {
            bufferBar.visibility = GONE
            return
        }
        bufferBar.visibility = VISIBLE
        // 块之间的竖线只是视觉分隔，投递时不变成任何字符（buffer.md 契约 #9）
        val body = if (s.bufferBlocks.isEmpty()) "（空）" else s.bufferBlocks.joinToString(" ▏")
        bufferText.text = "[${s.bufferStateLabel}] $body"
        bufferButtons.removeAllViews()
        fun add(label: String, action: BufferAction) {
            val b = baseButton(label, 13f) { actions.onBufferAction(action) }
            b.layoutParams = LayoutParams(0, LayoutParams.MATCH_PARENT, 1f)
            bufferButtons.addView(b)
        }
        if (s.bufferHeld) add("已确认", BufferAction.RELEASE_HOLD)
        if (s.bufferNeedsBind) add("绑定", BufferAction.BIND)
        add("下一块", BufferAction.SEND_NEXT)
        add("全部", BufferAction.SEND_ALL)
        add("删块", BufferAction.DELETE_LAST)
        add("清空", BufferAction.CLEAR)
    }

    // ── 九宫格拼音行 / 候选栏 ──

    private fun renderPinyinRow(items: List<String>) {
        pinyinScroll.visibility = if (items.isEmpty()) GONE else VISIBLE
        pinyinRow.removeAllViews()
        for (syl in items) {
            pinyinRow.addView(chip(syl, false) { actions.onPinSyllable(syl) })
        }
    }

    private fun renderCandidates(s: KeyboardState) {
        val key = Pair(s.candidateOffset, s.selected)
        if (s.candidates == lastCandidates && key == lastCandidateKey) return
        val reset = s.candidates != lastCandidates
        lastCandidates = s.candidates
        lastCandidateKey = key
        candRow.removeAllViews()
        s.candidates.forEachIndexed { i, text ->
            val abs = s.candidateOffset + i
            candRow.addView(chip(text, abs == s.selected) { actions.onCandidate(abs) })
        }
        if (reset) candScroll.scrollTo(0, 0)
    }

    private fun chip(text: String, highlighted: Boolean, onPress: () -> Unit): View {
        val tv = TextView(context)
        tv.text = text
        tv.textSize = 18f
        tv.gravity = Gravity.CENTER
        tv.setTextColor(if (highlighted) accent else fg)
        if (highlighted) tv.setTypeface(tv.typeface, Typeface.BOLD)
        tv.setPadding(dp(12), 0, dp(12), 0)
        tv.layoutParams = LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.MATCH_PARENT)
        guardClicks(tv, onPress)
        return tv
    }

    // ── 剪贴板面板 ──

    private fun renderClipboard(entries: List<ClipEntry>) {
        clipList.removeAllViews()
        val header = LinearLayout(context)
        header.orientation = HORIZONTAL
        val title = TextView(context)
        title.text = if (entries.isEmpty()) "剪贴板历史为空（仅在键盘显示时记录）" else "剪贴板历史"
        title.setTextColor(muted)
        title.setPadding(dp(10), dp(6), dp(10), dp(6))
        header.addView(title, LayoutParams(0, LayoutParams.WRAP_CONTENT, 1f))
        if (entries.isNotEmpty()) {
            val clear = baseButton("清空未收藏", 13f) { actions.onClipboardClear() }
            header.addView(clear, LayoutParams(LayoutParams.WRAP_CONTENT, dp(TOOLBAR_H)))
        }
        clipList.addView(header)
        for (e in entries) {
            val row = LinearLayout(context)
            row.orientation = HORIZONTAL
            val text = TextView(context)
            text.text = e.text.replace('\n', ' ')
            text.setSingleLine()
            text.ellipsize = TextUtils.TruncateAt.END
            text.textSize = 16f
            text.setTextColor(if (e.isLink) accent else fg)
            text.setPadding(dp(10), dp(8), dp(10), dp(8))
            guardClicks(text) { actions.onClipboardPick(e) }
            row.addView(text, LayoutParams(0, LayoutParams.WRAP_CONTENT, 1f))
            val pin = baseButton(if (e.pinned) "★" else "☆", 16f) { actions.onClipboardPin(e) }
            row.addView(pin, LayoutParams(dp(48), dp(TOOLBAR_H)))
            clipList.addView(row)
        }
    }

    // ── 键区 ──

    private fun buildKeys(layout: KeyboardLayout, shifted: Boolean) {
        keysArea.removeAllViews()
        when (layout) {
            KeyboardLayout.QWERTY -> buildQwerty(shifted)
            KeyboardLayout.SYMBOLS -> buildSymbols()
            KeyboardLayout.T9 -> buildT9()
            KeyboardLayout.NUMBER -> buildNumber()
        }
    }

    private fun buildQwerty(shifted: Boolean) {
        fun letters(s: String) = s.map { c ->
            val ch = if (shifted) c.uppercaseChar() else c
            charKey(ch)
        }
        keysArea.addView(row(letters("qwertyuiop")))
        keysArea.addView(row(listOf(spacer(0.5f)) + letters("asdfghjkl") + spacer(0.5f)))
        keysArea.addView(
            row(
                listOf(key(if (shifted) "⇪" else "⇧", 1.5f) { actions.onShift() }) +
                    letters("zxcvbnm") +
                    key("⌫", 1.5f) { actions.onBackspace() },
            ),
        )
        keysArea.addView(
            row(
                listOf(
                    key("123", 1.5f) { actions.onSwitchLayout(KeyboardLayout.SYMBOLS) },
                    charKey(','),
                    key("空格", 4f) { actions.onSpace() },
                    charKey('.'),
                    key("⏎", 1.5f) { actions.onEnter() },
                ),
            ),
        )
    }

    private fun buildSymbols() {
        keysArea.addView(row("1234567890".map { charKey(it) }))
        keysArea.addView(row("-/:;()$&@\"".map { charKey(it) }))
        keysArea.addView(row(".,?!'".map { charKey(it, 1.2f) } + key("⌫", 1.5f) { actions.onBackspace() }))
        keysArea.addView(
            row(
                listOf(
                    key("ABC", 1.5f) { actions.onSwitchLayout(KeyboardLayout.QWERTY) },
                    key("空格", 4f) { actions.onSpace() },
                    key("⏎", 1.5f) { actions.onEnter() },
                ),
            ),
        )
    }

    private fun buildT9() {
        fun digit(d: Char): View = key("$d\n${T9Input.LETTERS[d].orEmpty()}") { actions.onT9Digit(d) }
        keysArea.addView(row(listOf(key("1\n分词") { actions.onT9Separator() }, digit('2'), digit('3'))))
        keysArea.addView(row(listOf(digit('4'), digit('5'), digit('6'))))
        keysArea.addView(row(listOf(digit('7'), digit('8'), digit('9'))))
        keysArea.addView(
            row(
                listOf(
                    key("符号") { actions.onSwitchLayout(KeyboardLayout.SYMBOLS) },
                    key("空格", 1.6f) { actions.onSpace() },
                    key("⌫") { actions.onBackspace() },
                    key("⏎") { actions.onEnter() },
                ),
            ),
        )
    }

    private fun buildNumber() {
        keysArea.addView(row("123".map { charKey(it) }))
        keysArea.addView(row("456".map { charKey(it) }))
        keysArea.addView(row("789".map { charKey(it) }))
        keysArea.addView(row(listOf(charKey('.'), charKey('0'), key("⌫") { actions.onBackspace() })))
        keysArea.addView(row(listOf(charKey(','), key("空格") { actions.onSpace() }, key("⏎") { actions.onEnter() })))
    }

    private fun row(keys: List<View>): LinearLayout {
        val r = LinearLayout(context)
        r.orientation = HORIZONTAL
        keys.forEach { r.addView(it) }
        r.layoutParams = LayoutParams(LayoutParams.MATCH_PARENT, dp(KEY_H))
        return r
    }

    private fun spacer(weight: Float): View {
        val v = View(context)
        v.layoutParams = LayoutParams(0, LayoutParams.MATCH_PARENT, weight)
        return v
    }

    private fun charKey(ch: Char, weight: Float = 1f): View = key(ch.toString(), weight) { actions.onChar(ch) }

    private fun key(label: String, weight: Float = 1f, onPress: () -> Unit): View {
        val b = baseButton(label, if (label.length > 2) 15f else 19f, onPress)
        b.layoutParams = LayoutParams(0, LayoutParams.MATCH_PARENT, weight).apply {
            setMargins(dp(2), dp(3), dp(2), dp(3))
        }
        return b
    }

    private fun baseButton(label: String, sizeSp: Float, onPress: () -> Unit): Button {
        val b = Button(context)
        b.text = label
        b.isAllCaps = false
        b.setTextSize(TypedValue.COMPLEX_UNIT_SP, sizeSp)
        b.setTextColor(fg)
        b.minWidth = 0
        b.minimumWidth = 0
        b.minHeight = 0
        b.minimumHeight = 0
        b.setPadding(0, 0, 0, 0)
        guardClicks(b, onPress)
        return b
    }

    /**
     * 按下时记下输入框的「代」，点击触发时比对：换输入框、换模式后，旧的触摸流与排队的点击作废，
     * 迟到的抬起不会落到新输入框上（XSIME 吸收计划 §3.7）。
     */
    @SuppressLint("ClickableViewAccessibility")
    private fun guardClicks(v: View, onPress: () -> Unit) {
        var downGeneration = -1L
        v.setOnTouchListener { _, ev ->
            if (ev.actionMasked == MotionEvent.ACTION_DOWN) downGeneration = actions.currentGeneration()
            false
        }
        v.setOnClickListener {
            if (actions.isCurrent(downGeneration)) onPress()
        }
    }

    private fun dp(v: Int): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), resources.displayMetrics).toInt()

    private companion object {
        const val TOOLBAR_H = 40
        const val PREEDIT_H = 22
        const val CAND_H = 44
        const val KEY_H = 52
    }
}
