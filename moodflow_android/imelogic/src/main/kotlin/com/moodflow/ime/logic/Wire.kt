package com.moodflow.ime.logic

/**
 * 跨 JNI 的扁平编码的解码端，编码端是 `moodflow_android/rust/moodflow-android/src/wire.rs`。
 *
 * 结构化结果一律以 `String[]` 过边界：字段以 U+001F 分隔，**正文永远是最后一个字段**，
 * 按「最多切 N 段」切开，正文里即便出现分隔符也不会错位。两端格式改动必须同步。
 */
object Wire {
    const val US: Char = '\u001F'
    private const val RS: Char = '\u001E'

    /** 把一项切成最多 [n] 段；不足 [n] 段时补空串，调用方不必判长度。 */
    fun fields(item: String, n: Int): List<String> {
        val parts = item.split(US, limit = n)
        return if (parts.size >= n) parts else parts + List(n - parts.size) { "" }
    }

    /** 按键 / 选词 / 喂串的结果。`null` 或空数组按「未消费」处理。 */
    fun outcome(arr: Array<String>?): KeyOutcome {
        if (arr.isNullOrEmpty()) return KeyOutcome.PASSTHROUGH
        val flags = arr[0]
        val ops = ArrayList<EditOp>(arr.size - 1)
        for (i in 1 until arr.size) {
            decodeOp(arr[i])?.let(ops::add)
        }
        return KeyOutcome(
            consumed = 'c' in flags,
            modeChanged = 'm' in flags,
            ops = ops,
        )
    }

    private fun decodeOp(item: String): EditOp? {
        if (item.isEmpty()) return null
        val body = item.substring(1)
        return when (item[0]) {
            'C' -> EditOp.Commit(body)
            'S' -> {
                val (caret, text) = fields(body, 2)
                EditOp.SetComposition(text, caret.toIntOrNull() ?: text.length)
            }
            'D' -> body.toIntOrNull()?.let { EditOp.DeleteBackward(it) }
            'R' -> {
                val (count, text) = fields(body, 2)
                count.toIntOrNull()?.let { EditOp.ReplaceBackward(it, text) }
            }
            'M' -> body.toIntOrNull()?.let { EditOp.MoveCursor(it) }
            // 未知指令跳过而不是整条作废：新版核心多一种指令时，旧宿主至少还能执行认得的那些
            else -> null
        }
    }

    fun candidateWindow(arr: Array<String>?): CandidateWindow {
        if (arr.isNullOrEmpty()) return CandidateWindow.EMPTY
        val (offset, total, selected) = fields(arr[0], 3)
        return CandidateWindow(
            offset = offset.toIntOrNull() ?: 0,
            total = total.toIntOrNull() ?: 0,
            selected = selected.toIntOrNull() ?: 0,
            items = arr.drop(1),
        )
    }

    fun schemas(arr: Array<String>?): List<SchemaEntry> =
        arr.orEmpty().map {
            val (id, name, icon, active) = fields(it, 4)
            SchemaEntry(id, name, icon, active == "1")
        }

    /** 主题与双拼布局同一形状：`id US 名称 US 是否激活`。 */
    fun namedEntries(arr: Array<String>?): List<NamedEntry> =
        arr.orEmpty().map {
            val (id, name, active) = fields(it, 3)
            NamedEntry(id, name, active == "1")
        }

    fun clipEntries(arr: Array<String>?): List<ClipEntry> =
        arr.orEmpty().mapNotNull {
            val (id, kind, pinned, text) = fields(it, 4)
            val parsed = id.toLongOrNull() ?: return@mapNotNull null
            ClipEntry(parsed, kind == "l", pinned == "1", text)
        }

    fun configRegistry(arr: Array<String>?): List<ConfigField> =
        arr.orEmpty().map {
            val (key, ty, options) = fields(it, 3)
            ConfigField(key, ty, if (options.isEmpty()) emptyList() else options.split(RS))
        }

    /** 并击结算结果：`null` 未结算；`N` 无映射；`Es…` 音节 / `Ef…` 片段。 */
    fun chord(s: String?): ChordResult? = when {
        s.isNullOrEmpty() -> null
        s == "N" -> ChordResult.NoMatch
        s.length >= 2 && s[0] == 'E' -> ChordResult.Emit(s.substring(2), fragment = s[1] == 'f')
        else -> null
    }

    /** 字符串列表配置值的编码（与 Rust 侧 `parse_config_value` 的类型 4 对应）。 */
    fun joinList(values: List<String>): String = values.joinToString(US.toString())
}

/** 对宿主文本区的一次编辑（与核心 `moodflow_host::EditOp` 一一对应）。 */
sealed interface EditOp {
    data class Commit(val text: String) : EditOp

    /** [caret] 是组字串内的 UTF-16 偏移（Rust 侧已从字节偏移换算）。空文本 = 清除组字。 */
    data class SetComposition(val text: String, val caret: Int) : EditOp

    /** 删除光标前 [count] 个**码点**。 */
    data class DeleteBackward(val count: Int) : EditOp

    /** 删除光标前 [count] 个码点后插入 [text]，宿主应包在同一个批量编辑里。 */
    data class ReplaceBackward(val count: Int, val text: String) : EditOp

    data class MoveCursor(val delta: Int) : EditOp
}

data class KeyOutcome(
    val consumed: Boolean,
    val modeChanged: Boolean,
    val ops: List<EditOp>,
) {
    companion object {
        val PASSTHROUGH = KeyOutcome(consumed = false, modeChanged = false, ops = emptyList())
    }
}

data class CandidateWindow(
    val offset: Int,
    val total: Int,
    /** 当前高亮候选的**绝对**下标 */
    val selected: Int,
    val items: List<String>,
) {
    companion object {
        val EMPTY = CandidateWindow(0, 0, 0, emptyList())
    }
}

data class SchemaEntry(val id: String, val name: String, val iconLabel: String, val active: Boolean)

data class NamedEntry(val id: String, val name: String, val active: Boolean)

data class ClipEntry(val id: Long, val isLink: Boolean, val pinned: Boolean, val text: String)

data class ConfigField(val key: String, val type: String, val options: List<String>)

sealed interface ChordResult {
    data class Emit(val pinyin: String, val fragment: Boolean) : ChordResult
    data object NoMatch : ChordResult
}
