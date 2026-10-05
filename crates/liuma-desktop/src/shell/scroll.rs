//! 全高轨道滚动条 handle。
//!
//! 组件库把轨道自身高当滚动视口,轨道挂 content-card 全高时差值段
//! (底部栈/状态栏)成盲区——content_size 加 extra = (轨道高 − 列表
//! 视口高) 补偿,thumb 满行程映射回列表真实滚动域(轨道高由渲染期
//! canvas 捕获存 store)。行程域取列表**真实可滚量**
//! (`max_offset_for_scrollbar`):组件库以 content_size ≤ 轨道高判定
//! 「内容未溢出」并整轴隐藏(滚动中也不画),行程必须计入。位置与
//! 拖拽直接委托 ListState 的像素域(`scroll_px_offset_for_scrollbar` /
//! `set_offset_from_scrollbar`):值域 [−max_offset, 0] 与组件行程域
//! 精确重合,thumb 随内容连续滑动、拖拽按像素跟随;钉底跟随态库侧
//! 返回满偏移(thumb 停在轨道底),拖拽期间库侧冻结 max_offset 防漂移。

use gpui_kit::component::scroll::ScrollbarHandle;
use gpui_kit::{Bounds, ListState, Pixels, Point, Size, px};

/// 全高轨道 + 像素域连续映射 handle
#[derive(Clone)]
pub struct FullTrackHandle {
    list: ListState,
    /// 轨道高 − 列表视口高(渲染期捕获,上一帧值;布局稳定后收敛)
    extra: Pixels,
}

impl FullTrackHandle {
    pub fn new(list: &ListState, extra: Pixels) -> Self {
        Self {
            list: list.clone(),
            extra: extra.max(px(0.)),
        }
    }

    /// thumb 行程 = 列表真实可滚量;0 即不可滚(组件库按内容未溢出隐藏)
    fn travel(&self) -> f32 {
        f32::from(self.list.max_offset_for_scrollbar().y).max(0.)
    }
}

impl ScrollbarHandle for FullTrackHandle {
    fn offset(&self) -> Point<Pixels> {
        self.list.scroll_px_offset_for_scrollbar()
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        self.list.set_offset_from_scrollbar(offset);
    }

    fn content_size(&self) -> Size<Pixels> {
        // 高 = 轨道高(视口 + 全列差值补偿)+ 真实行程:组件库以
        // content_size ≤ 轨道高整轴隐藏,行程必须计入;宽度沿用视口
        let size = self.list.viewport_bounds().size;
        Size::new(
            size.width,
            px(f32::from(size.height) + f32::from(self.extra) + self.travel()),
        )
    }

    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.list.viewport_bounds()
    }
}
