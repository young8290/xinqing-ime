package com.moodflow.ime.logic

/** 宿主文本区（`InputConnection` 的薄抽象，便于 JVM 单测）。 */
interface TextSink {
    fun setComposing(text: String, caret: Int)
    fun commit(text: String)
    fun deleteBefore(codePoints: Int)
    fun replaceBefore(codePoints: Int, text: String)
    fun moveCursor(delta: Int)
}

/** Buffer 的入口（`NativeBuffer` 的薄抽象）。返回是否收下（超容量等会拒收）。 */
interface BufferSink {
    fun pushPhrase(text: String): Boolean
}

/**
 * 把核心的编辑指令派发到宿主或 Buffer。
 *
 * Buffer 开启时（XSIME 吸收计划 §2.1「确认的候选先进 Buffer 而不是宿主」）：
 * - 上屏 → 进 Buffer，成为一个块；
 * - 组字 → 不写进宿主（编码栏在候选区自绘），只清掉宿主里可能残留的组字；
 * - 删除 / 替换 / 移动光标 → 丢弃：它们针对的是宿主里**已有**的文本，Buffer 模式下
 *   那些文本不是本次输入的产物，改动它们会误伤用户原文。
 */
object OpRouter {
    /** @return 被 Buffer 拒收的上屏文本（调用方提示用户；不静默丢弃也不改投宿主）。 */
    fun apply(ops: List<EditOp>, text: TextSink, buffer: BufferSink?): List<String> {
        val rejected = ArrayList<String>()
        for (op in ops) {
            if (buffer == null) {
                when (op) {
                    is EditOp.Commit -> text.commit(op.text)
                    is EditOp.SetComposition -> text.setComposing(op.text, op.caret)
                    is EditOp.DeleteBackward -> text.deleteBefore(op.count)
                    is EditOp.ReplaceBackward -> text.replaceBefore(op.count, op.text)
                    is EditOp.MoveCursor -> text.moveCursor(op.delta)
                }
            } else {
                when (op) {
                    is EditOp.Commit -> if (op.text.isNotEmpty() && !buffer.pushPhrase(op.text)) {
                        rejected += op.text
                    }
                    is EditOp.SetComposition -> if (op.text.isEmpty()) text.setComposing("", 0)
                    is EditOp.DeleteBackward, is EditOp.ReplaceBackward, is EditOp.MoveCursor -> Unit
                }
            }
        }
        return rejected
    }
}
