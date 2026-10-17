// Moodflow Android（输入法应用）。
//
// 两个模块：
// - :imelogic —— 纯 Kotlin/JVM，不碰 Android SDK：字段隐私判据、跨 JNI 编码的解码、
//   键码映射、九宫格输入状态、编辑指令派发。单测在任何有 JDK 的机器上都能跑。
// - :app —— InputMethodService、键盘视图、设置页、JNI 包装，以及经 cargo-ndk 编出的
//   libmoodflow_android.so。
//
// 没有 Android SDK 的机器（如只想跑 :imelogic 单测）加 `-Pmoodflow.jvmOnly=true`：
// 此时不引入 Android Gradle 插件、也不包含 :app。
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "moodflow-android"

include(":imelogic")
if (providers.gradleProperty("moodflow.jvmOnly").orNull != "true") {
    include(":app")
}
