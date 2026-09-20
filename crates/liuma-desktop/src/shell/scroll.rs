//! 全高轨道滚动条 handle(条目序号比例映射)。
//!
//! 两层问题在此收口:
//! 1. 「滚动条不能拉到底部」:组件库把轨道自身高当滚动视口,轨道挂
//!    content-card 全高时差值段(底部栈/状态栏)成盲区——content_size
//!    加 extra = (轨道高 − 列表视口高) 补偿,thumb 满行程映射回真实
//!    滚动域(轨道高由渲染期 canvas 捕获存 store)。
//! 2. 「thumb 忽长忽短/到顶跳中间」:thumb 位置此前由**测高像素**决定,
//!    而虚拟化列表按可见性测高(视口外 0 高),滚动/跳转时新行被测出
//!    → 总高与偏移双变 → thumb 跳变。改为**条目序号比例域**:offset
//!    与 content 都按「条数 × 固定行高」构造,thumb 位置 = 顶行序号 /
//!    总条数,与测高完全解耦——连续性由构造保证;条数只在内容增删时
//!    变化,单调无跳变。拖拽经同一比例域反向映射回 scroll_to。
//!    列表自身的滚轮/跟随逻辑仍走真实像素(局部正确),仅滚动条的
//!    显示映射换轨。

use gpui_kit::component::scroll::ScrollbarHandle;
use gpui_kit::{Bounds, ListState, Pixels, Point, Size, px};

/// 比例域的固定行高(仅决定 thumb 视觉长度比例,与真实行高无关)
const ROW_UNIT: f32 = 64.;

/// 全高轨道 + 条目比例映射 handle
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

    /// 比例域总高(条数 × 固定行高 + 轨道差补偿)
    fn virtual_height(&self) -> f32 {
        self.list.item_count() as f32 * ROW_UNIT + f32::from(self.extra)
    }

    /// 顶行序号(钉底跟随态 logical=None → 条数 = 比例 1)
    fn top_item_ix(&self) -> usize {
        self.list.logical_scroll_top().item_ix
    }
}

impl ScrollbarHandle for FullTrackHandle {
    fn offset(&self) -> Point<Pixels> {
        // offset = −(top/count) × extent:thumb 位置 = 纯逻辑位置
        let count = self.list.item_count();
        let container = f32::from(self.list.viewport_bounds().size.height);
        let extent = (self.virtual_height() - container).max(0.);
        let frac = if count == 0 {
            0.
        } else {
            (self.top_item_ix() as f32 / count as f32).clamp(0., 1.)
        };
        Point::new(px(0.), px(-frac * extent))
    }

    fn set_offset(&self, offset: Point<Pixels>) {
        // 拖拽反向映射:offset 比例 → 顶行序号(顶对齐)
        let count = self.list.item_count();
        if count == 0 {
            return;
        }
        let container = f32::from(self.list.viewport_bounds().size.height);
        let extent = (self.virtual_height() - container).max(0.);
        if extent <= 0. {
            return;
        }
        let frac = (-f32::from(offset.y) / extent).clamp(0., 1.);
        self.list.scroll_to(gpui_kit::ListOffset {
            item_ix: (frac * count as f32).round() as usize,
            offset_in_item: px(0.),
        });
    }

    fn content_size(&self) -> Size<Pixels> {
        // 宽度沿用视口宽;高度走比例域(与测高解耦)
        Size::new(
            self.list.viewport_bounds().size.width,
            px(self.virtual_height()),
        )
    }

    fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.list.viewport_bounds()
    }
}
