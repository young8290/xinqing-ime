# JNI：Rust 侧按名字调用这些方法（sink.rs），按名字注册入口（lib.rs）。
-keep class com.moodflow.ime.jni.** { *; }
-keepclasseswithmembernames class * {
    native <methods>;
}
