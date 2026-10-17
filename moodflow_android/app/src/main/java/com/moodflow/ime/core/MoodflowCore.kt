package com.moodflow.ime.core

import com.moodflow.ime.jni.MoodflowNative as N
import com.moodflow.ime.jni.NativeEventSink
import com.moodflow.ime.logic.CandidateWindow
import com.moodflow.ime.logic.ChordResult
import com.moodflow.ime.logic.ClipEntry
import com.moodflow.ime.logic.KeyOutcome
import com.moodflow.ime.logic.NamedEntry
import com.moodflow.ime.logic.SchemaEntry
import com.moodflow.ime.logic.StrokeTable
import com.moodflow.ime.logic.Target
import com.moodflow.ime.logic.Wire
import java.io.File

/**
 * 输入法核心会话（`moodflow_mobile::MobileCore` 的 Kotlin 包装）。
 *
 * 构造耗时（加载词库、编译方案），必须在后台线程调 [create]；其余方法在主线程调用。
 * [close] 之后不得再用。
 */
class MoodflowCore private constructor(private var handle: Long) : AutoCloseable {

    fun keyDown(vk: Int, modifiers: Int): KeyOutcome = Wire.outcome(N.nativeKeyDown(handle, vk, modifiers))

    fun focusGained(isPrivate: Boolean) = N.nativeFocusGained(handle, isPrivate)

    fun focusLost() = N.nativeFocusLost(handle)

    fun runCommand(cmd: Command, intArg: Int = 0, strArg: String = "") =
        N.nativeRunCommand(handle, cmd.code, intArg, strArg)

    fun userStoreAvailable(): Boolean = N.nativeUserStoreAvailable(handle)

    fun schemas(): List<SchemaEntry> = Wire.schemas(N.nativeSchemas(handle))

    fun candidates(offset: Int, limit: Int): CandidateWindow =
        Wire.candidateWindow(N.nativeCandidates(handle, offset, limit))

    fun selectCandidate(index: Int): KeyOutcome = Wire.outcome(N.nativeSelectCandidate(handle, index))

    fun themes(): List<NamedEntry> = Wire.namedEntries(N.nativeThemes(handle))

    fun shuangpinLayouts(): List<NamedEntry> = Wire.namedEntries(N.nativeShuangpinLayouts(handle))

    fun charStrokes(): StrokeTable = StrokeTable.fromFlat(N.nativeCharStrokes(handle))

    fun configSnapshot(): String = N.nativeConfigSnapshot(handle).orEmpty()

    /** @return null 成功，否则为给用户看的失败原因 */
    fun configSet(key: String, type: ConfigType, value: String): String? =
        N.nativeConfigSet(handle, key, type.code, value)

    fun feedText(text: String): KeyOutcome = Wire.outcome(N.nativeFeedText(handle, text))

    fun replaceComposition(text: String): KeyOutcome = Wire.outcome(N.nativeReplaceComposition(handle, text))

    fun clearComposition(): KeyOutcome = Wire.outcome(N.nativeClearComposition(handle))

    @Synchronized
    override fun close() {
        if (handle != 0L) {
            N.nativeDestroy(handle)
            handle = 0L
        }
    }

    /** 与 Rust 侧 `nativeRunCommand` 的 `cmd` 编码一致。 */
    enum class Command(val code: Int) {
        ENGLISH(0), SELECT_SCHEMA(1), TOGGLE_PUNCT(2), TOGGLE_WIDTH(3), TOGGLE_S2T(4),
        RELOAD_CONFIG(5), SET_SHUANGPIN_LAYOUT(6),
    }

    /** 与 Rust 侧 `parse_config_value` 的类型编码一致。 */
    enum class ConfigType(val code: Int) { BOOL(0), INT(1), FLOAT(2), STR(3), STR_LIST(4) }

    companion object {
        /** 后台线程调用。.so 加载失败或构造失败返回 null（键盘退化为直接上屏）。 */
        fun create(dataDir: File, userRoot: File, fallbackSchema: String, sink: NativeEventSink): MoodflowCore? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeCreate(dataDir.absolutePath, userRoot.absolutePath, fallbackSchema, sink)
            return if (h == 0L) null else MoodflowCore(h)
        }
    }
}

/** 九宫格切分（无状态）。 */
object T9Native {
    fun segmentations(input: String, limit: Int): List<String> =
        if (input.isEmpty() || !N.ensureLoaded()) emptyList()
        else N.nativeT9Segmentations(input, limit)?.toList().orEmpty()

    fun leading(input: String): List<String> =
        if (input.isEmpty() || !N.ensureLoaded()) emptyList()
        else N.nativeT9Leading(input)?.toList().orEmpty()

    fun pin(input: String, syllable: String): String? =
        if (!N.ensureLoaded()) null else N.nativeT9Pin(input, syllable)
}

/** Buffer（`moodflow_mobile::buffer::MobileBuffer`）。凭据留在 Rust 侧，这里只报目标与结果。 */
class NativeBuffer private constructor(private var h: Long) : AutoCloseable {
    enum class State { UNBOUND, READY, SUSPENDED, HELD }
    enum class SendKey { CONFIRM_COMPOSITION, SEND_NEXT, PASS_THROUGH }
    enum class WriteResult(val code: Int) { ACCEPTED(0), REJECTED(1), UNKNOWN(2) }

    fun bind(t: Target, isPrivate: Boolean): Boolean = N.nativeBufferBind(h, t.id, t.revision, isPrivate)
    fun focusChanged(t: Target) = N.nativeBufferFocusChanged(h, t.id, t.revision)
    fun suspend() = N.nativeBufferSuspend(h)
    fun discard() = N.nativeBufferDiscard(h)
    fun releaseHold() = N.nativeBufferReleaseHold(h)

    /** 返回 0 成功 / 1 空 / 2 超容量 / 3 块在投递中。 */
    fun pushPhrase(text: String): Int = N.nativeBufferPushPhrase(h, text)
    fun pushLatin(codePoint: Int): Int = N.nativeBufferPushLatin(h, codePoint)
    fun backspace(): Int = N.nativeBufferBackspace(h)
    fun deleteLast(): Int = N.nativeBufferDeleteLast(h)
    fun clear() = N.nativeBufferClear(h)
    fun blocks(): List<String> = N.nativeBufferBlocks(h)?.toList().orEmpty()

    fun state(): State = when (N.nativeBufferState(h)) {
        1 -> State.READY
        2 -> State.SUSPENDED
        3 -> State.HELD
        else -> State.UNBOUND
    }

    fun sendKeyAction(composing: Boolean): SendKey = when (N.nativeBufferSendKeyAction(h, composing)) {
        0 -> SendKey.CONFIRM_COMPOSITION
        1 -> SendKey.SEND_NEXT
        else -> SendKey.PASS_THROUGH
    }

    /** 取出待投递文本；不能投递时返回 null。 */
    fun begin(t: Target, all: Boolean): String? = N.nativeBufferBegin(h, t.id, t.revision, all)
    fun isAuthorized(t: Target): Boolean = N.nativeBufferAuthorized(h, t.id, t.revision)
    fun finish(r: WriteResult): Boolean = N.nativeBufferFinish(h, r.code)

    @Synchronized
    override fun close() {
        if (h != 0L) {
            N.nativeBufferFree(h)
            h = 0L
        }
    }

    companion object {
        fun create(): NativeBuffer? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeBufferNew(0)
            return if (h == 0L) null else NativeBuffer(h)
        }
    }
}

/** 剪贴板历史（`moodflow_mobile::clipboard::MobileClipboard`），落盘到 [file]。 */
class NativeClipboard private constructor(private var h: Long) : AutoCloseable {
    fun record(text: String, sensitive: Boolean, nowSecs: Long): Int = N.nativeClipRecord(h, text, sensitive, nowSecs)
    fun entries(query: String = ""): List<ClipEntry> = Wire.clipEntries(N.nativeClipEntries(h, query))
    fun setPinned(id: Long, pinned: Boolean): Boolean = N.nativeClipSetPinned(h, id, pinned)
    fun remove(id: Long): Boolean = N.nativeClipRemove(h, id)
    fun clearUnpinned() = N.nativeClipClearUnpinned(h)

    @Synchronized
    override fun close() {
        if (h != 0L) {
            N.nativeClipFree(h)
            h = 0L
        }
    }

    companion object {
        fun open(file: File, nowSecs: Long): NativeClipboard? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeClipOpen(file.absolutePath, nowSecs)
            return if (h == 0L) null else NativeClipboard(h)
        }
    }
}

/** 物理键盘并击结算器（`moodflow_mobile::chord::MobileChord`）。 */
class NativeChord private constructor(private var h: Long) : AutoCloseable {
    fun isChordKey(key: String): Boolean = N.nativeChordIsChordKey(h, key)
    fun keyDown(key: String, nowMs: Long): ChordResult? = Wire.chord(N.nativeChordKeyDown(h, key, nowMs))
    fun keyUp(key: String, nowMs: Long): ChordResult? = Wire.chord(N.nativeChordKeyUp(h, key, nowMs))
    fun tick(nowMs: Long): ChordResult? = Wire.chord(N.nativeChordTick(h, nowMs))
    fun reset() = N.nativeChordReset(h)

    @Synchronized
    override fun close() {
        if (h != 0L) {
            N.nativeChordFree(h)
            h = 0L
        }
    }

    companion object {
        /** 校验并构造；不可应用时返回 null（原因用 [validate] 取）。 */
        fun fromJson(json: String): NativeChord? {
            if (!N.ensureLoaded()) return null
            val h = N.nativeChordNew(json)
            return if (h == 0L) null else NativeChord(h)
        }

        /** @return null = 可以应用，否则为原因 */
        fun validate(json: String): String? =
            if (!N.ensureLoaded()) "原生库未加载" else N.nativeChordValidate(json)
    }
}
