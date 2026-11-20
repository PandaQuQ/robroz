//! Small canvas drawings: check mark, checkbox, toggle, the brand rings.
//! Sizes and colours are passed in from the skin; nothing is fixed here.

use iced::mouse;
use iced::widget::canvas::{self, Frame, Geometry, Path, Program, Stroke};
use iced::{Color, Point, Rectangle, Renderer, Size, Theme as IcedTheme};

pub struct Check {
    pub color: Color,
    pub stroke: f32,
}

impl<M> Program<M> for Check {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &IcedTheme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h, s) = (bounds.width, bounds.height, self.stroke / 2.0);
        let path = Path::new(|b| {
            b.move_to(Point::new(s, h * 0.5));
            b.line_to(Point::new(w * 0.36, h - s));
            b.line_to(Point::new(w - s, s));
        });
        frame.stroke(
            &path,
            Stroke::default()
                .with_color(self.color)
                .with_width(self.stroke)
                .with_line_cap(canvas::LineCap::Round)
                .with_line_join(canvas::LineJoin::Round),
        );
        vec![frame.into_geometry()]
    }
}

pub struct CheckBox {
    pub checked: bool,
    pub fill: Color,
    pub mark: Color,
    pub border: Color,
    pub border_width: f32,
    pub radius: f32,
    pub mark_stroke: f32,
}

impl<M> Program<M> for CheckBox {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &IcedTheme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        if self.checked {
            let rect = Path::rounded_rectangle(Point::ORIGIN, size, self.radius.into());
            frame.fill(&rect, self.fill);
            let (w, h) = (size.width, size.height);
            let mark = Path::new(|b| {
                b.move_to(Point::new(w * 0.24, h * 0.52));
                b.line_to(Point::new(w * 0.43, h * 0.70));
                b.line_to(Point::new(w * 0.77, h * 0.32));
            });
            frame.stroke(
                &mark,
                Stroke::default()
                    .with_color(self.mark)
                    .with_width(self.mark_stroke)
                    .with_line_cap(canvas::LineCap::Round)
                    .with_line_join(canvas::LineJoin::Round),
            );
        } else {
            let i = self.border_width / 2.0;
            let rect = Path::rounded_rectangle(
                Point::new(i, i),
                Size::new(size.width - 2.0 * i, size.height - 2.0 * i),
                self.radius.into(),
            );
            frame.stroke(
                &rect,
                Stroke::default()
                    .with_color(self.border)
                    .with_width(self.border_width),
            );
        }
        vec![frame.into_geometry()]
    }
}

pub struct Toggle {
    pub on: bool,
    pub track_on: Color,
    pub track_off: Color,
    pub knob: Color,
    pub knob_size: f32,
}

impl<M> Program<M> for Toggle {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &IcedTheme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let track = Path::rounded_rectangle(Point::ORIGIN, size, (size.height / 2.0).into());
        frame.fill(
            &track,
            if self.on {
                self.track_on
            } else {
                self.track_off
            },
        );
        let r = self.knob_size / 2.0;
        let inset = (size.height - self.knob_size) / 2.0;
        let cx = if self.on {
            size.width - inset - r
        } else {
            inset + r
        };
        frame.fill(
            &Path::circle(Point::new(cx, size.height / 2.0), r),
            self.knob,
        );
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        if cursor.is_over(bounds) {
            mouse::Interaction::Pointer
        } else {
            mouse::Interaction::default()
        }
    }
}

/// Two outline rings side by side (the brand mark, SPEC §7 empty state).
pub struct Rings {
    pub color: Color,
    pub radius: f32,
    pub stroke: f32,
}

impl Rings {
    pub fn size(&self) -> Size {
        let d = 2.0 * self.radius + self.stroke;
        Size::new(2.0 * d + self.stroke, d)
    }
}

impl<M> Program<M> for Rings {
    type State = ();

    fn draw(
        &self,
        _: &(),
        renderer: &Renderer,
        _: &IcedTheme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let d = 2.0 * self.radius + self.stroke;
        let cy = bounds.height / 2.0;
        for cx in [d / 2.0, d * 1.5 + self.stroke] {
            frame.stroke(
                &Path::circle(Point::new(cx, cy), self.radius),
                Stroke::default()
                    .with_color(self.color)
                    .with_width(self.stroke),
            );
        }
        vec![frame.into_geometry()]
    }
}
