package com.moodflow.ime.logic

/** 宿主眼中「是哪个输入框」的原料（取自 `EditorInfo`）。 */
data class FieldKey(val packageName: String, val fieldId: Int, val inputType: Int)

/** Buffer 的目标（与 Rust 侧 `moodflow_buffer::Target` 对应）。 */
data class Target(val id: Long, val revision: Long)

/** 一次 `onStartInput` 对 Buffer 意味着什么（`docs/design/buffer.md` §3 的 Android 一列）。 */
enum class FieldChange {
    /** 换了输入框，或离开过再回来：丢弃 Buffer 内容与绑定。 */
    NEW_FIELD,

    /** 同一输入框的上下文重启（`restarting = true`，修订号递增）：暂停授权、保留内容，需重新绑定。 */
    SAME_FIELD_RESTART,
}

/**
 * 跟踪当前输入框身份、修订号与「代」。
 *
 * - **修订号**：每次 `onStartInput` 递增。同一字段重启后旧投递授权必然失效（Rust 侧比对 id+修订号）。
 * - **代**（[generation]）：每次开始或结束输入都递增。键盘视图捕获点击时记下代，处理时代不一致
 *   就丢弃——换输入框后迟到的点击、排队的抬起不会落到新输入框上（XSIME 吸收计划 §3.7）。
 */
class FieldTracker {
    private var lastKey: FieldKey? = null
    private var active = false
    private var revision = 0L

    var generation: Long = 0
        private set

    var current: Target? = null
        private set

    fun onStartInput(key: FieldKey, restarting: Boolean): FieldChange {
        val same = active && restarting && key == lastKey
        lastKey = key
        active = true
        revision += 1
        generation += 1
        current = Target(stableId(key), revision)
        return if (same) FieldChange.SAME_FIELD_RESTART else FieldChange.NEW_FIELD
    }

    /** `onFinishInput`：离开输入框。之后再进来（哪怕同一个）都算新字段。 */
    fun onFinishInput() {
        active = false
        generation += 1
        current = null
    }

    fun isCurrent(gen: Long): Boolean = gen == generation

    companion object {
        /** 稳定的 63 位正数 id（FNV-1a）：同一字段多次进入得到同一个 id。 */
        fun stableId(key: FieldKey): Long {
            var h = -0x340d631b7bdddcdbL // FNV offset basis 0xcbf29ce484222325
            val s = "${key.packageName}\u0000${key.fieldId}\u0000${key.inputType}"
            for (ch in s) {
                h = h xor ch.code.toLong()
                h *= 0x100000001b3L
            }
            return h and Long.MAX_VALUE
        }
    }
}
