package com.moodflow.ime.logic

/**
 * 核心的按键入口吃 Windows 虚拟键码（`moodflow-keys` 的 VK 常量 SSOT），修饰键位见
 * `moodflow_host::Modifiers`。这里只放宿主要用到的那几个，数值与 `keymap.rs` 逐一对应。
 */
object Vk {
    const val BACK = 0x08
    const val TAB = 0x09
    const val RETURN = 0x0D
    const val ESCAPE = 0x1B
    const val SPACE = 0x20
    const val PRIOR = 0x21
    const val NEXT = 0x22
    const val LEFT = 0x25
    const val UP = 0x26
    const val RIGHT = 0x27
    const val DOWN = 0x28
    const val DELETE = 0x2E
    const val KEY_0 = 0x30
    const val KEY_A = 0x41
    const val SEMICOLON = 0xBA
    const val EQUAL = 0xBB
    const val COMMA = 0xBC
    const val MINUS = 0xBD
    const val PERIOD = 0xBE
    const val SLASH = 0xBF
    const val BACKTICK = 0xC0
    const val LBRACKET = 0xDB
    const val BACKSLASH = 0xDC
    const val RBRACKET = 0xDD
    const val QUOTE = 0xDE
}

/** `moodflow_host::Modifiers` 的位。 */
object Mods {
    const val NONE = 0
    const val SHIFT = 0x0001
    const val CTRL = 0x0002
    const val ALT = 0x0004
}

/**
 * Android 物理键盘 → 核心 VK / 并击键名。
 *
 * 键码与修饰位照抄 `android.view.KeyEvent` 的公开常量，本对象不依赖 Android SDK。
 */
object PhysicalKeys {
    // android.view.KeyEvent
    const val KEYCODE_0 = 7
    const val KEYCODE_9 = 16
    const val KEYCODE_DPAD_UP = 19
    const val KEYCODE_DPAD_DOWN = 20
    const val KEYCODE_DPAD_LEFT = 21
    const val KEYCODE_DPAD_RIGHT = 22
    const val KEYCODE_A = 29
    const val KEYCODE_Z = 54
    const val KEYCODE_COMMA = 55
    const val KEYCODE_PERIOD = 56
    const val KEYCODE_TAB = 61
    const val KEYCODE_SPACE = 62
    const val KEYCODE_ENTER = 66
    const val KEYCODE_DEL = 67
    const val KEYCODE_GRAVE = 68
    const val KEYCODE_MINUS = 69
    const val KEYCODE_EQUALS = 70
    const val KEYCODE_LEFT_BRACKET = 71
    const val KEYCODE_RIGHT_BRACKET = 72
    const val KEYCODE_BACKSLASH = 73
    const val KEYCODE_SEMICOLON = 74
    const val KEYCODE_APOSTROPHE = 75
    const val KEYCODE_SLASH = 76
    const val KEYCODE_PAGE_UP = 92
    const val KEYCODE_PAGE_DOWN = 93
    const val KEYCODE_ESCAPE = 111
    const val KEYCODE_FORWARD_DEL = 112

    const val META_SHIFT_ON = 0x1
    const val META_ALT_ON = 0x2
    const val META_CTRL_ON = 0x1000

    /** 键码 → VK；不归输入法管的键（音量、媒体…）返回 `null`，宿主交还系统。 */
    fun toVk(keyCode: Int): Int? = when (keyCode) {
        in KEYCODE_A..KEYCODE_Z -> Vk.KEY_A + (keyCode - KEYCODE_A)
        in KEYCODE_0..KEYCODE_9 -> Vk.KEY_0 + (keyCode - KEYCODE_0)
        KEYCODE_SPACE -> Vk.SPACE
        KEYCODE_ENTER -> Vk.RETURN
        KEYCODE_DEL -> Vk.BACK
        KEYCODE_FORWARD_DEL -> Vk.DELETE
        KEYCODE_TAB -> Vk.TAB
        KEYCODE_ESCAPE -> Vk.ESCAPE
        KEYCODE_COMMA -> Vk.COMMA
        KEYCODE_PERIOD -> Vk.PERIOD
        KEYCODE_GRAVE -> Vk.BACKTICK
        KEYCODE_MINUS -> Vk.MINUS
        KEYCODE_EQUALS -> Vk.EQUAL
        KEYCODE_LEFT_BRACKET -> Vk.LBRACKET
        KEYCODE_RIGHT_BRACKET -> Vk.RBRACKET
        KEYCODE_BACKSLASH -> Vk.BACKSLASH
        KEYCODE_SEMICOLON -> Vk.SEMICOLON
        KEYCODE_APOSTROPHE -> Vk.QUOTE
        KEYCODE_SLASH -> Vk.SLASH
        KEYCODE_DPAD_UP -> Vk.UP
        KEYCODE_DPAD_DOWN -> Vk.DOWN
        KEYCODE_DPAD_LEFT -> Vk.LEFT
        KEYCODE_DPAD_RIGHT -> Vk.RIGHT
        KEYCODE_PAGE_UP -> Vk.PRIOR
        KEYCODE_PAGE_DOWN -> Vk.NEXT
        else -> null
    }

    /** `KeyEvent.getMetaState()` → `Modifiers` 位。 */
    fun toMods(metaState: Int): Int {
        var m = Mods.NONE
        if (metaState and META_SHIFT_ON != 0) m = m or Mods.SHIFT
        if (metaState and META_CTRL_ON != 0) m = m or Mods.CTRL
        if (metaState and META_ALT_ON != 0) m = m or Mods.ALT
        return m
    }

    /**
     * 键码 → 并击键名（与 `moodflow-keys` 的 `KEY_TABLE.names` 逐字一致：字母小写，
     * 符号键用 `semicolon` / `comma` / `period` / `slash` / `quote` / `backtick`…）。
     */
    fun chordKeyName(keyCode: Int): String? = when (keyCode) {
        in KEYCODE_A..KEYCODE_Z -> ('a' + (keyCode - KEYCODE_A)).toString()
        KEYCODE_SEMICOLON -> "semicolon"
        KEYCODE_COMMA -> "comma"
        KEYCODE_PERIOD -> "period"
        KEYCODE_SLASH -> "slash"
        KEYCODE_APOSTROPHE -> "quote"
        KEYCODE_GRAVE -> "backtick"
        KEYCODE_SPACE -> "space"
        else -> null
    }
}

/** 一个字符在键盘上的按法（来自核心的 `char_strokes`）。 */
data class Stroke(val vk: Int, val shift: Boolean)

/**
 * 键面字符 → 按法。软键盘只知道「用户点了哪个字符」，引擎入口却是键事件；这张表由核心给
 * （`MobileCore::char_strokes`），宿主不自己维护第二份——自己维护的那份漏掉的字符会绕过引擎
 * 直接上屏，智能标点、配对、中英标点切换对它全部失效。
 */
class StrokeTable private constructor(private val map: Map<Char, Stroke>) {
    operator fun get(ch: Char): Stroke? = map[ch]

    val size: Int get() = map.size

    companion object {
        val EMPTY = StrokeTable(emptyMap())

        /** 解码 `[码点, VK, 是否 Shift, …]` 平铺数组；BMP 外的码点跳过（键面上没有）。 */
        fun fromFlat(flat: IntArray?): StrokeTable {
            if (flat == null) return EMPTY
            val m = HashMap<Char, Stroke>(flat.size / 3)
            var i = 0
            while (i + 2 < flat.size) {
                val cp = flat[i]
                if (cp in 0..0xFFFF) {
                    m.putIfAbsent(cp.toChar(), Stroke(flat[i + 1], flat[i + 2] != 0))
                }
                i += 3
            }
            return StrokeTable(m)
        }
    }
}
