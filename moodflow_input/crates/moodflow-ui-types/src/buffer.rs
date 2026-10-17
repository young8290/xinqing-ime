//! Buffer 浮窗的视图数据（docs/design/buffer.md）。

/// 协调器交给渲染端的 Buffer 浮窗内容。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BufferView {
    /// 攒着的正文（块之间直接拼接，不加空格）。
    pub text: String,
    /// 状态行：绑定与否、可用的操作提示。
    pub status: String,
    /// 光标所在行的屏幕坐标（左上）与行高，浮窗据此贴在行的上方。
    pub caret_x: i32,
    pub caret_y: i32,
    pub caret_height: i32,
}
