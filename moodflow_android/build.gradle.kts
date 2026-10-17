// 插件放在根 buildscript 的 classpath 上，子模块用不带版本的 `plugins { id(...) }` 取用：
// Kotlin 插件只加载一次（:imelogic 与 :app 共用），且 kotlin-android 能看见同一份 AGP。
//
// 用 buildscript 而不是 `plugins { ... apply false }`，是为了能按 `moodflow.jvmOnly`
// 不加载 AGP（见 settings.gradle.kts）——plugins 块不能带条件。
buildscript {
    val jvmOnly = (project.findProperty("moodflow.jvmOnly") as String?) == "true"
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
    dependencies {
        classpath("org.jetbrains.kotlin:kotlin-gradle-plugin:2.0.21")
        if (!jvmOnly) {
            classpath("com.android.tools.build:gradle:8.7.3")
        }
    }
}
