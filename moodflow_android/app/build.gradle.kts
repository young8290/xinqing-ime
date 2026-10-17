import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// 版本取 moodflow_android/VERSION（XSIME 吸收计划 §4.1：主次版本号与 docs/VERSION 一致，
// 补丁号可独立递增；标签 android-vX.Y.Z）。versionCode = 主×1e6 + 次×1e3 + 补丁。
val appVersion: String = rootProject.file("VERSION").readText().trim()
val appVersionCode: Int = appVersion.split(".").let { parts ->
    fun at(i: Int) = parts.getOrNull(i)?.takeWhile(Char::isDigit)?.toIntOrNull() ?: 0
    at(0) * 1_000_000 + at(1) * 1_000 + at(2)
}

// cargo-ndk 产出的 .so 与从仓库 data/ 拷来的出厂数据都放在 build/ 下，不入库。
val rustJniDir: File = layout.buildDirectory.dir("rustJniLibs").get().asFile
val generatedAssetsDir: File = layout.buildDirectory.dir("generated/moodflowAssets").get().asFile

android {
    namespace = "com.moodflow.ime"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.moodflow.ime"
        minSdk = 26
        targetSdk = 35
        versionCode = appVersionCode
        versionName = appVersion
        ndk {
            abiFilters += listOf("arm64-v8a", "x86_64")
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDir(rustJniDir)
            assets.srcDir(generatedAssetsDir)
        }
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(project(":imelogic"))
    testImplementation("junit:junit:4.13.2")
}

// ── Rust：cargo-ndk 交叉编译 libmoodflow_android.so ─────────────────────────
//
// 需要 cargo、cargo-ndk、rustup 目标（aarch64-linux-android / x86_64-linux-android）与
// ANDROID_NDK_HOME。只想编 Kotlin 时加 -Pmoodflow.skipRust=true（APK 里就没有 .so，
// 键盘会退化成直接上屏）。-Pmoodflow.abis=arm64-v8a 可只编一个 ABI。
val rustAbis: List<String> = (providers.gradleProperty("moodflow.abis").orNull ?: "arm64-v8a,x86_64")
    .split(",").map { it.trim() }.filter { it.isNotEmpty() }
val skipRust: Boolean = providers.gradleProperty("moodflow.skipRust").orNull == "true"

val cargoBuild = tasks.register<Exec>("cargoBuild") {
    group = "build"
    description = "用 cargo-ndk 编译 libmoodflow_android.so"
    workingDir = rootProject.file("rust")
    val cmd = mutableListOf("cargo", "ndk")
    rustAbis.forEach { cmd += listOf("-t", it) }
    cmd += listOf("-P", "26", "-o", rustJniDir.absolutePath, "build", "--release", "-p", "moodflow-android")
    commandLine(cmd)
    // 增量交给 cargo 自己判断
    outputs.upToDateWhen { false }
}

// ── 出厂数据：仓库根 data/ → APK assets/data（首次启动由 DataInstaller 解到 filesDir/data）──
val copyMoodflowData = tasks.register<Sync>("copyMoodflowData") {
    description = "把仓库根 data/ 拷进 APK assets"
    from(rootProject.file("../data"))
    into(File(generatedAssetsDir, "data"))
}

// 除了挂在 preBuild 上，再显式挂到真正消费这两个目录的合并任务上：Gradle 8 会拒绝
// 「用了别的任务的输出却没声明依赖」的任务，不能只靠 preBuild 的传递顺序。
val assetConsumers = Regex("(merge|package)\\w*Assets")
val jniConsumers = Regex("merge\\w*JniLibFolders")
tasks.configureEach {
    if (name == "preBuild" || assetConsumers.matches(name)) {
        dependsOn(copyMoodflowData)
    }
    if (!skipRust && (name == "preBuild" || jniConsumers.matches(name))) {
        dependsOn(cargoBuild)
    }
}
