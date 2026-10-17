//! 反向通道：核心的 UiCommand 泵线程 → Kotlin `NativeEventSink`。
//!
//! 回调发生在核心的泵线程上（**不是**宿主主线程），Kotlin 侧自行切回主线程。
//! 泵线程常驻，故永久挂接到 JVM；每次回调包一层 local frame，局部引用不累积。
//!
//! 方法签名必须与 `NativeEventSink.kt` 逐字一致——写错只在运行期报 `NoSuchMethodError`，
//! 这里一旦失败只记一条不含正文的日志并清掉 Java 异常，不让它带走泵线程。

use jni::objects::{GlobalRef, JObject, JValue};
use jni::{JNIEnv, JavaVM};
use moodflow_mobile::{CandidateFrame, InputStatus, MobileEventSink};

use crate::wire;

pub struct JniSink {
    vm: JavaVM,
    target: GlobalRef,
}

impl JniSink {
    pub fn new(vm: JavaVM, target: GlobalRef) -> Self {
        Self { vm, target }
    }

    fn call(
        &self,
        what: &'static str,
        f: impl FnOnce(&mut JNIEnv, &JObject) -> jni::errors::Result<()>,
    ) {
        let Ok(mut env) = self.vm.attach_current_thread_permanently() else {
            tracing::warn!("sink {what}: 无法挂接 JVM");
            return;
        };
        let target = self.target.as_obj();
        let r = env.with_local_frame(16, |env| f(env, target));
        if r.is_err() {
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            tracing::warn!("sink {what}: 回调失败");
        }
    }
}

impl MobileEventSink for JniSink {
    fn on_candidates(&self, frame: CandidateFrame) {
        self.call("onCandidates", |env, target| {
            let caret = wire::utf8_to_utf16_offset(&frame.preedit, frame.preedit_caret);
            let preedit = env.new_string(&frame.preedit)?;
            let label = env.new_string(&frame.mode_label)?;
            let items = env.new_object_array(
                frame.items.len() as i32,
                "java/lang/String",
                JObject::null(),
            )?;
            for (i, item) in frame.items.iter().enumerate() {
                let s = env.new_string(item)?;
                env.set_object_array_element(&items, i as i32, &s)?;
            }
            env.call_method(
                target,
                "onCandidates",
                "(Ljava/lang/String;ILjava/lang/String;[Ljava/lang/String;III)V",
                &[
                    JValue::Object(&preedit),
                    JValue::Int(caret as i32),
                    JValue::Object(&label),
                    JValue::Object(&items),
                    JValue::Int(frame.selected as i32),
                    JValue::Int(frame.page as i32),
                    JValue::Int(frame.total_pages as i32),
                ],
            )?;
            Ok(())
        });
    }

    fn on_hide_candidates(&self) {
        self.call("onHideCandidates", |env, target| {
            env.call_method(target, "onHideCandidates", "()V", &[])?;
            Ok(())
        });
    }

    fn on_status(&self, status: InputStatus) {
        self.call("onStatus", |env, target| {
            let label = env.new_string(&status.icon_label)?;
            env.call_method(
                target,
                "onStatus",
                "(ZLjava/lang/String;ZZZZ)V",
                &[
                    JValue::Bool(status.chinese_mode.into()),
                    JValue::Object(&label),
                    JValue::Bool(status.full_width.into()),
                    JValue::Bool(status.chinese_punct.into()),
                    JValue::Bool(status.s2t_enabled.into()),
                    JValue::Bool(status.s2t_shown.into()),
                ],
            )?;
            Ok(())
        });
    }

    fn on_toast(&self, text: String) {
        self.call("onToast", |env, target| {
            let s = env.new_string(&text)?;
            env.call_method(
                target,
                "onToast",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&s)],
            )?;
            Ok(())
        });
    }
}
