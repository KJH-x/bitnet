use iced::{
    advanced::{
        layout, mouse, overlay, renderer,
        widget::{operation, tree, Id, Tree},
        Clipboard, Layout, Shell, Widget,
    },
    time::Instant,
    widget::{scrollable, Scrollable},
    window, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector,
};
use std::time::Duration;

const LINE_DISTANCE: f32 = 84.0;
const MAX_PENDING_DISTANCE: f32 = 4.0 * LINE_DISTANCE;
const ANIMATION: Duration = Duration::from_millis(120);
const FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Default)]
struct WheelState {
    motion: Option<Motion>,
}

struct Motion {
    from: f32,
    to: f32,
    elapsed: Duration,
    last_frame: Instant,
}

impl WheelState {
    fn queue(&mut self, current: f32, distance: f32, limit: f32, now: Instant) -> f32 {
        let remaining = self
            .motion
            .as_ref()
            .map_or(0.0, |motion| motion.to - current);
        let inherited = if remaining.signum() == distance.signum() {
            remaining
        } else {
            0.0
        };
        let requested = (inherited + distance).clamp(-MAX_PENDING_DISTANCE, MAX_PENDING_DISTANCE);
        let target = (current + requested).clamp(0.0, limit);
        let motion = Motion {
            from: current,
            to: target,
            elapsed: Duration::from_millis(8),
            last_frame: now,
        };
        let position = motion.position();
        self.motion = Some(motion);
        position
    }

    fn advance(&mut self, now: Instant) -> Option<f32> {
        let motion = self.motion.as_mut()?;
        let since_last = now.saturating_duration_since(motion.last_frame);
        // A delayed frame still gets an intermediate position instead of
        // moving the full remaining distance in one jump.
        motion.last_frame = now;
        motion.elapsed += since_last.min(2 * FRAME_INTERVAL);
        let position = motion.position();
        if motion.elapsed >= ANIMATION {
            self.motion = None;
        }
        Some(position)
    }
}

impl Motion {
    fn position(&self) -> f32 {
        let remaining = 1.0 - (self.elapsed.as_secs_f32() / ANIMATION.as_secs_f32()).min(1.0);
        self.to - (self.to - self.from) * remaining.powi(3)
    }
}

/// Acts only on the wrapped native scrollable, leaving its descendants alone.
#[derive(Default)]
struct ScrollY {
    current: f32,
    limit: f32,
    set: Option<f32>,
}

impl operation::Operation for ScrollY {
    fn traverse(&mut self, _: &mut dyn FnMut(&mut dyn operation::Operation)) {}

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        self.current = translation.y;
        self.limit = (content_bounds.height - bounds.height).max(0.0);
        if let Some(position) = self.set {
            state.scroll_to(operation::scrollable::AbsoluteOffset {
                x: None,
                y: Some(position.clamp(0.0, self.limit)),
            });
        }
    }
}

/// Keeps the native viewport, touchpad, touch and scrollbar behavior. Both the
/// line-wheel and frame handling use widget state without an application update.
pub(super) fn vertical<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
) -> SmoothScroll<'a, Message> {
    SmoothScroll {
        native: scrollable(content),
    }
}

pub(super) struct SmoothScroll<'a, Message> {
    native: Scrollable<'a, Message>,
}

impl<Message> SmoothScroll<'_, Message> {
    pub(super) fn height(mut self, height: impl Into<Length>) -> Self {
        self.native = self.native.height(height);
        self
    }

    pub(super) fn width(mut self, width: impl Into<Length>) -> Self {
        self.native = self.native.width(width);
        self
    }

    pub(super) fn scrollbar_spacing(mut self, spacing: f32) -> Self {
        self.native = self.native.direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::new().spacing(spacing),
        ));
        self
    }

    fn scroll_y(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        set: Option<f32>,
    ) -> ScrollY {
        let mut operation = ScrollY {
            set,
            ..ScrollY::default()
        };
        self.native.operate(tree, layout, renderer, &mut operation);
        operation
    }
}

impl<Message> Widget<Message, Theme, Renderer> for SmoothScroll<'_, Message> {
    fn size(&self) -> Size<Length> {
        self.native.size()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<WheelState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(WheelState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(
            &self.native as &dyn Widget<Message, Theme, Renderer>,
        )]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.native as &dyn Widget<Message, Theme, Renderer>]);
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.native.layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.native.draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<WheelState>();
        let child = &mut tree.children[0];
        match event {
            Event::Window(window::Event::RedrawRequested(now)) => {
                if let Some(position) = state.advance(*now) {
                    self.scroll_y(child, layout, renderer, Some(position));
                    if state.motion.is_some() {
                        shell.request_redraw_at(*now + FRAME_INTERVAL);
                    }
                }
            }
            Event::Mouse(
                mouse::Event::ButtonPressed(_)
                | mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels { .. },
                },
            )
            | Event::Touch(_)
            | Event::Window(window::Event::Unfocused) => {
                state.motion = None;
            }
            _ => {}
        }

        let line_delta = match event {
            Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: _, y },
            }) if y.is_finite() && *y != 0.0 && cursor.is_over(layout.bounds()) => Some(*y),
            _ => None,
        };
        let previous = line_delta.map(|_| self.scroll_y(child, layout, renderer, None));

        self.native.update(
            child, event, layout, cursor, renderer, clipboard, shell, viewport,
        );

        if let (Some(previous), Some(lines)) = (previous, line_delta) {
            let actual = self.scroll_y(child, layout, renderer, None);
            // If the child consumed the event, native routing did not move this
            // viewport. Horizontal wheel and Shift+wheel also stay native.
            if (actual.current - previous.current) * lines < 0.0 {
                let position = state.queue(
                    previous.current,
                    -lines * LINE_DISTANCE,
                    actual.limit,
                    Instant::now(),
                );
                self.scroll_y(child, layout, renderer, Some(position));
                shell.capture_event();
                shell.request_redraw();
            }
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn operation::Operation,
    ) {
        self.native
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.native
            .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        self.native.overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message: 'a> From<SmoothScroll<'a, Message>> for Element<'a, Message> {
    fn from(scrollable: SmoothScroll<'a, Message>) -> Self {
        Element::new(scrollable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::{advanced::clipboard, widget::Space, Font, Pixels, Point};

    #[test]
    fn wheel_moves_immediately_and_settles_over_multiple_frames() {
        let start = Instant::now();
        let mut state = WheelState::default();
        let first = state.queue(200.0, LINE_DISTANCE, 500.0, start);
        assert!(first > 200.0 && first < 284.0);
        let mut previous = first;
        for frame in 1..=7 {
            let offset = state.advance(start + frame * FRAME_INTERVAL).unwrap();
            assert!(offset > previous && offset <= 284.0);
            previous = offset;
        }
        assert_eq!(previous, 284.0);
        assert!(state.motion.is_none());
    }

    #[test]
    fn rapid_wheel_input_accumulates_then_reverses_from_the_visible_position() {
        let start = Instant::now();
        let mut state = WheelState::default();
        let first = state.queue(200.0, LINE_DISTANCE, 500.0, start);
        let next = state.queue(first, LINE_DISTANCE, 500.0, start);
        assert_eq!(state.motion.as_ref().unwrap().to, 368.0);
        let reversed = state.queue(next, -LINE_DISTANCE, 500.0, start);
        assert!(reversed < next);
        assert_eq!(state.motion.as_ref().unwrap().to, next - LINE_DISTANCE);
    }

    #[test]
    fn widget_animates_line_input_and_preserves_direct_pixel_input() {
        let mut scroll = vertical::<()>(Space::new().width(120).height(600))
            .width(180)
            .height(120)
            .scrollbar_spacing(8.0);
        let renderer = Renderer::new(Font::DEFAULT, Pixels(16.0));
        let mut tree = Tree::new(&scroll as &dyn Widget<(), Theme, Renderer>);
        let bounds = Size::new(180.0, 120.0);
        let node = scroll.layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, bounds),
        );
        let layout = Layout::new(&node);
        let viewport = Rectangle::with_size(bounds);
        let cursor = mouse::Cursor::Available(Point::new(15.0, 15.0));
        let mut clipboard = clipboard::Null;
        let mut messages = Vec::new();
        let mut shell = Shell::new(&mut messages);

        scroll.update(
            &mut tree,
            &Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Lines { x: 0.0, y: -1.0 },
            }),
            layout,
            cursor,
            &renderer,
            &mut clipboard,
            &mut shell,
            &viewport,
        );
        assert!(shell.is_event_captured());
        assert!(shell.is_empty());
        let first = scroll
            .scroll_y(&mut tree.children[0], layout, &renderer, None)
            .current;
        assert!(first > 0.0 && first < LINE_DISTANCE);

        let mut shell = Shell::new(&mut messages);
        scroll.update(
            &mut tree,
            &Event::Window(window::Event::RedrawRequested(
                Instant::now() + FRAME_INTERVAL,
            )),
            layout,
            cursor,
            &renderer,
            &mut clipboard,
            &mut shell,
            &viewport,
        );
        assert!(shell.is_empty());
        let next = scroll
            .scroll_y(&mut tree.children[0], layout, &renderer, None)
            .current;
        assert!(next > first && next < LINE_DISTANCE);

        scroll.update(
            &mut tree,
            &Event::Mouse(mouse::Event::WheelScrolled {
                delta: mouse::ScrollDelta::Pixels { x: 0.0, y: -24.0 },
            }),
            layout,
            cursor,
            &renderer,
            &mut clipboard,
            &mut shell,
            &viewport,
        );
        assert_eq!(
            scroll
                .scroll_y(&mut tree.children[0], layout, &renderer, None)
                .current,
            next + 24.0
        );
        assert!(tree.state.downcast_ref::<WheelState>().motion.is_none());
    }
}
