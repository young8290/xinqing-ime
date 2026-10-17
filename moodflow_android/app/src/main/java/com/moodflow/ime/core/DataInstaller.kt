package com.moodflow.ime.core

import android.content.Context
import android.content.res.AssetManager
import android.os.Build
import android.util.Log
import java.io.File

/**
 * 把 APK 里的出厂数据（`assets/data`，构建时拷自仓库根 `data/`）解到 `filesDir/data`。
 *
 * 目录关系是核心定死的：`data_dir` 必须是 `<安装根>/data`（见 `moodflow_mobile::mount_dirs`），
 * 这里安装根 = `filesDir`；用户数据根是 `filesDir/rust`，与出厂数据分开，重装出厂数据不碰用户数据。
 *
 * 只在 APK 更新后重解一次（以 versionCode + 安装时间为标记）。整目录先删后写：出厂数据目录
 * 不存放任何用户文件，旧版留下的文件若不删，会被核心当成仍然存在的方案。
 */
object DataInstaller {
    private const val TAG = "Moodflow"
    private const val MARKER = ".installed"

    fun dataDir(ctx: Context): File = File(ctx.filesDir, "data")

    fun userRoot(ctx: Context): File = File(ctx.filesDir, "rust").apply { mkdirs() }

    /** 后台线程调用。返回出厂数据目录。 */
    fun install(ctx: Context): File {
        val dir = dataDir(ctx)
        val stamp = stamp(ctx)
        val marker = File(dir, MARKER)
        if (marker.isFile && marker.readText() == stamp) return dir
        dir.deleteRecursively()
        dir.mkdirs()
        val count = copyTree(ctx.assets, "data", dir)
        marker.writeText(stamp)
        Log.i(TAG, "出厂数据已解包：$count 个文件")
        return dir
    }

    private fun stamp(ctx: Context): String {
        val info = if (Build.VERSION.SDK_INT >= 33) {
            ctx.packageManager.getPackageInfo(ctx.packageName, android.content.pm.PackageManager.PackageInfoFlags.of(0))
        } else {
            @Suppress("DEPRECATION")
            ctx.packageManager.getPackageInfo(ctx.packageName, 0)
        }
        @Suppress("DEPRECATION")
        val code = if (Build.VERSION.SDK_INT >= 28) info.longVersionCode else info.versionCode.toLong()
        return "$code:${info.lastUpdateTime}"
    }

    /** 递归拷贝；`list` 返回空数组的路径按文件处理。 */
    private fun copyTree(assets: AssetManager, path: String, dest: File): Int {
        val children = assets.list(path).orEmpty()
        if (children.isEmpty()) {
            dest.parentFile?.mkdirs()
            return try {
                assets.open(path).use { input -> dest.outputStream().use { input.copyTo(it) } }
                1
            } catch (e: java.io.IOException) {
                // 空目录在 assets 里也表现为「没有子项」，open 会失败：跳过即可
                0
            }
        }
        dest.mkdirs()
        var n = 0
        for (child in children) {
            n += copyTree(assets, "$path/$child", File(dest, child))
        }
        return n
    }
}
