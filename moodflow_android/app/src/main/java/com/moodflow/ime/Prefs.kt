package com.moodflow.ime

import android.content.Context
import android.content.SharedPreferences

/** 宿主侧偏好（引擎配置走核心的配置读写，不在这里）。 */
object Prefs {
    private const val FILE = "moodflow"

    /** 中文字母区默认用九宫格 */
    const val T9_DEFAULT = "t9_default"

    /** 剪贴板历史（仅键盘可见时采集） */
    const val CLIPBOARD_HISTORY = "clipboard_history"

    /** 已应用的并击方案（filesDir 下） */
    const val CHORD_FILE = "chord_keymap.json"

    fun of(ctx: Context): SharedPreferences = ctx.getSharedPreferences(FILE, Context.MODE_PRIVATE)
}
