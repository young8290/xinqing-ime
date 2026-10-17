package com.moodflow.ime

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.text.InputType
import android.util.TypedValue
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import android.widget.Toast
import com.moodflow.ime.core.NativeChord
import java.io.File

/**
 * 最小设置页：启用 / 切换输入法、宿主侧开关、并击方案导入。
 *
 * 引擎配置（候选数、模糊音…）走核心的配置读写（`MoodflowCore.configSet` + 重载），
 * 完整的设置界面是后续工作；这一页只放 Android 宿主自己的开关。
 */
class SettingsActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val prefs = Prefs.of(this)
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(16), dp(16), dp(16), dp(16))
        }

        root.addView(text("Moodflow 输入法 ${versionName()}", 20f))
        root.addView(text("先在系统设置里启用，再切换为当前输入法。", 14f))
        root.addView(button("启用输入法") { startActivity(Intent(Settings.ACTION_INPUT_METHOD_SETTINGS)) })
        root.addView(
            button("切换输入法") {
                getSystemService(InputMethodManager::class.java)?.showInputMethodPicker()
            },
        )

        root.addView(text("键盘", 16f))
        root.addView(
            switch("中文默认用九宫格", prefs.getBoolean(Prefs.T9_DEFAULT, false)) {
                prefs.edit().putBoolean(Prefs.T9_DEFAULT, it).apply()
            },
        )
        root.addView(
            switch("剪贴板历史（只在键盘显示时记录，密码框不记录）", prefs.getBoolean(Prefs.CLIPBOARD_HISTORY, true)) {
                prefs.edit().putBoolean(Prefs.CLIPBOARD_HISTORY, it).apply()
            },
        )

        root.addView(text("物理键盘并击（键位方案 JSON，格式见 moodflow-chord）", 16f))
        val chordFile = File(filesDir, Prefs.CHORD_FILE)
        val editor = EditText(this).apply {
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_FLAG_MULTI_LINE or
                InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
            minLines = 4
            setText(if (chordFile.isFile) chordFile.readText() else "")
        }
        root.addView(editor)
        root.addView(
            button("校验并应用（重新打开键盘后生效）") {
                val json = editor.text.toString()
                val err = NativeChord.validate(json)
                if (err == null) {
                    chordFile.writeText(json)
                    toast("已应用")
                } else {
                    toast("不能应用：$err")
                }
            },
        )
        root.addView(
            button("停用并击") {
                chordFile.delete()
                editor.setText("")
                toast("已停用")
            },
        )

        setContentView(ScrollView(this).apply { addView(root) })
    }

    private fun versionName(): String = try {
        val info = if (Build.VERSION.SDK_INT >= 33) {
            packageManager.getPackageInfo(packageName, android.content.pm.PackageManager.PackageInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            packageManager.getPackageInfo(packageName, 0)
        }
        info.versionName.orEmpty()
    } catch (e: Exception) {
        ""
    }

    private fun text(s: String, sizeSp: Float) = TextView(this).apply {
        text = s
        setTextSize(TypedValue.COMPLEX_UNIT_SP, sizeSp)
        setPadding(0, dp(12), 0, dp(4))
    }

    private fun button(label: String, onClick: () -> Unit) = Button(this).apply {
        text = label
        isAllCaps = false
        setOnClickListener { onClick() }
    }

    @Suppress("DEPRECATION")
    private fun switch(label: String, checked: Boolean, onChange: (Boolean) -> Unit) = Switch(this).apply {
        text = label
        isChecked = checked
        setOnCheckedChangeListener { _, value -> onChange(value) }
    }

    private fun toast(s: String) = Toast.makeText(this, s, Toast.LENGTH_SHORT).show()

    private fun Context.dp(v: Int): Int =
        TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, v.toFloat(), resources.displayMetrics).toInt()
}
