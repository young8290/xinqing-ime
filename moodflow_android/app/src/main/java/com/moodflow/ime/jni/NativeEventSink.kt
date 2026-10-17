package com.moodflow.ime.jni

/**
 * 核心的反向通道（候选、状态、提示），由 Rust 侧 `sink.rs` 按方法名 + JNI 签名调用。
 *
 * ⚠ 回调发生在核心的泵线程上，**不是**主线程；实现方自行切回主线程。
 * 方法名与参数类型改动必须同步 `sink.rs` 里的签名串。
 */
interface NativeEventSink {
    /** 签名 `(Ljava/lang/String;ILjava/lang/String;[Ljava/lang/String;III)V` */
    fun onCandidates(
        preedit: String,
        caret: Int,
        modeLabel: String,
        items: Array<String>,
        selected: Int,
        page: Int,
        totalPages: Int,
    )

    /** 签名 `()V` */
    fun onHideCandidates()

    /** 签名 `(ZLjava/lang/String;ZZZZ)V` */
    fun onStatus(
        chineseMode: Boolean,
        iconLabel: String,
        fullWidth: Boolean,
        chinesePunct: Boolean,
        s2tEnabled: Boolean,
        s2tShown: Boolean,
    )

    /** 签名 `(Ljava/lang/String;)V` */
    fun onToast(text: String)
}
