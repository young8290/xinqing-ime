package com.moodflow.ime.jni

import java.io.File
import java.lang.reflect.Modifier
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * JNI 契约对账：Rust 侧导出的 `Java_com_moodflow_ime_jni_MoodflowNative_*` 与本类的 `external fun`
 * 名字、参数类型、返回类型逐一相同。
 *
 * JNI 名字或签名写错**不会**编译失败，只会在设备上调用那一刻抛 `UnsatisfiedLinkError`——CI 没有设备，
 * 这条测试是唯一能在 CI 里抓住它的地方。
 */
class MoodflowNativeContractTest {
    private data class Sig(val params: List<String>, val ret: String)

    private fun rustSource(): String {
        val candidates = listOf(
            File("../rust/moodflow-android/src/lib.rs"),
            File("rust/moodflow-android/src/lib.rs"),
            File("moodflow_android/rust/moodflow-android/src/lib.rs"),
        )
        val f = candidates.firstOrNull { it.isFile }
            ?: error("找不到 rust/moodflow-android/src/lib.rs（工作目录 ${File(".").absolutePath}）")
        return f.readText()
    }

    private fun rustType(t: String): String = when (t.trim()) {
        "jlong" -> "long"
        "jint" -> "int"
        "jboolean" -> "boolean"
        "JString" -> "java.lang.String"
        "JObject" -> "object"
        "jstring" -> "java.lang.String"
        "jobjectArray" -> "java.lang.String[]"
        "jintArray" -> "int[]"
        else -> error("未登记的 JNI 类型: $t")
    }

    private fun rustExports(): Map<String, Sig> {
        val re = Regex(
            """pub extern "system" fn Java_com_moodflow_ime_jni_MoodflowNative_(\w+)\(([^)]*)\)\s*(?:->\s*(\w+))?""",
        )
        return re.findAll(rustSource()).associate { m ->
            val params = m.groupValues[2].split(',')
                .map { it.trim() }
                .filter { it.isNotEmpty() }
                .map { it.substringAfter(':').trim() }
                .drop(2) // JNIEnv, JClass
                .map(::rustType)
            val ret = m.groupValues[3].let { if (it.isEmpty()) "void" else rustType(it) }
            m.groupValues[1] to Sig(params, ret)
        }
    }

    private fun javaType(c: Class<*>): String = when {
        c.isPrimitive -> c.name
        c.isArray -> javaType(c.componentType!!) + "[]"
        c == String::class.java -> "java.lang.String"
        else -> "object"
    }

    private fun kotlinExports(): Map<String, Sig> =
        MoodflowNative::class.java.declaredMethods
            .filter { Modifier.isNative(it.modifiers) && Modifier.isStatic(it.modifiers) }
            .associate { m ->
                m.name to Sig(m.parameterTypes.map(::javaType), javaType(m.returnType).let {
                    if (it == "void") "void" else it
                })
            }

    @Test
    fun everyKotlinExternalHasAMatchingRustExport() {
        val rust = rustExports()
        val kotlin = kotlinExports()
        assertTrue("Rust 侧一个导出都没解析到，正则过期了？", rust.size > 10)
        assertTrue("Kotlin 侧一个 static native 都没找到", kotlin.size > 10)
        assertEquals("名字集合不一致", kotlin.keys.sorted(), rust.keys.sorted())
        for ((name, sig) in kotlin) {
            assertEquals("$name 的签名", rust.getValue(name), sig)
        }
    }
}
