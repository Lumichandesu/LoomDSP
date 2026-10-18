//! SPIKE #186: a layer over the whole window that draws the drag line from
//! the source port to the pointer as a row of small quads. It reports no
//! mouse interaction, so the stack does not hide the cursor from the layers
//! below it.

use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::widget::{self, Widget};
use iced::{Border, Color, Element, Length, Point, Rectangle, Size, Theme};

pub struct DragLine {
    pub start: Point,
    pub end: Point,
}

impl<Message, R: renderer::Renderer> Widget<Message, Theme, R> for DragLine {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut widget::Tree, _renderer: &R, limits: &layout::Limits) -> layout::Node {
        layout::atomic(limits, Length::Fill, Length::Fill)
    }

    fn draw(
        &self,
        _tree: &widget::Tree,
        renderer: &mut R,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: iced::mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let d = self.end - self.start;
        let len = (d.x * d.x + d.y * d.y).sqrt();
        let steps = (len / 6.0).ceil().max(1.0) as usize;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = Point::new(self.start.x + d.x * t, self.start.y + d.y * t);
            renderer.fill_quad(
                Quad {
                    bounds: Rectangle::new(Point::new(p.x - 2.0, p.y - 2.0), Size::new(4.0, 4.0)),
                    border: Border::default().rounded(2.0),
                    ..Quad::default()
                },
                Color::from_rgb(0.95, 0.6, 0.2),
            );
        }
    }
}

impl<'a, Message: 'a, R: renderer::Renderer + 'a> From<DragLine> for Element<'a, Message, Theme, R> {
    fn from(line: DragLine) -> Self {
        Element::new(line)
    }
}
