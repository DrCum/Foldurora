//! Pure monitor bindings; deliberately test-only until real IO/surface callers exist.
//! Port of MonitorSet/CyclingMonitor in fssessionmonitormodel.h and
//! fssessionmonitortools.h, pinned Legacy source abbe3cb0ca66ee2484f277219f6a27706fab7fc6.
//! Four fixed views and a cycler borrow canonical ChatSurface slots. This module
//! owns no chat, image, renderer, endpoint, native command or rate reservation.
//! Explicit Open of Active and binding survival on activation are approved staged
//! differences: the first producer remembers only ordinary authorized Active frames.

use super::chat_ui::ChatSurface;
use super::slots::SlotId;
use aurora_session::chat::ChatSource;
use aurora_session::preview_image::PreviewSource;
use aurora_session::{MAX_WORKERS, PreviewRate, PreviewSize};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);
const FIXED: usize = MAX_WORKERS - 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Error {
    Invalid,
    Unavailable,
    Capacity,
    Busy,
    Clock,
    Exhausted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum View {
    Fixed1,
    Fixed2,
    Fixed3,
    Fixed4,
    Cycling,
}

impl View {
    pub const ALL: [Self; 5] = [Self::Fixed1, Self::Fixed2, Self::Fixed3, Self::Fixed4, Self::Cycling];

    pub fn surface(self) -> ChatSurface {
        match self {
            Self::Fixed1 => ChatSurface::Monitor1,
            Self::Fixed2 => ChatSurface::Monitor2,
            Self::Fixed3 => ChatSurface::Monitor3,
            Self::Fixed4 => ChatSurface::Monitor4,
            Self::Cycling => ChatSurface::Cycling,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Fixed1 => 0,
            Self::Fixed2 => 1,
            Self::Fixed3 => 2,
            Self::Fixed4 => 3,
            Self::Cycling => 4,
        }
    }
}

/// Caller publishes actual current managed source facts, independent of image
/// availability or conversation permissions. Slot labels are never consulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Source {
    slot: SlotId,
    chat: ChatSource,
    preview: PreviewSource,
    available: bool,
}

impl Source {
    pub fn new(slot: SlotId, chat: ChatSource, available: bool) -> Result<Self, Error> {
        chat.validate().map_err(|_| Error::Invalid)?;
        let preview = PreviewSource::new(chat.worker, chat.session, chat.lease_epoch).map_err(|_| Error::Invalid)?;
        Ok(Self {
            slot,
            chat,
            preview,
            available,
        })
    }

    pub fn slot(self) -> SlotId {
        self.slot
    }
    pub fn chat(self) -> ChatSource {
        self.chat
    }
    pub fn preview(self) -> PreviewSource {
        self.preview
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Options {
    pub size: PreviewSize,
    pub rate: PreviewRate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Binding {
    pub source: Source,
    pub incarnation: u64,
    pub enabled: bool,
    pub visible: bool,
    pub options: Options,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Direction {
    Previous,
    Next,
}

/// Captured at press from a committed source binding. Native focus/modal/close
/// owners call cancel_input; no release may choose a new source or list index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Ticket {
    instance: u64,
    input: u64,
    view: View,
    binding: Binding,
}

impl Ticket {
    pub fn source(self) -> Source {
        self.binding.source
    }
    pub fn view(self) -> View {
        self.view
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Navigation {
    origin: Ticket,
    target: Source,
}

impl Navigation {
    pub fn target(self) -> Source {
        self.target
    }
}

/// Opaque local correlation, not an activation request or acknowledgement.
/// Shell must correlate its actual Coordinator outcome to this exact value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Exchange {
    instance: u64,
    operation: u64,
    origin: Ticket,
    previous: Option<Source>,
}

impl Exchange {
    pub fn target(self) -> Source {
        self.origin.source()
    }
    pub fn previous(self) -> Option<Source> {
        self.previous
    }
    pub fn view(self) -> View {
        self.origin.view()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExchangeResult {
    Ignored,
    Kept,
    Exchanged,
    Stopped,
}

/// Local binding transition only. It cannot represent Send/Read/Review/Activate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Changed {
    pub view: View,
    pub previous: Source,
    pub current: Source,
}

pub(super) struct MonitorSet {
    instance: u64,
    serial: u64,
    input: u64,
    exhausted: bool,
    sources: Vec<Source>,
    order: [SlotId; MAX_WORKERS],
    active: Option<PreviewSource>,
    bindings: [Option<Binding>; 5],
    pending: Option<Exchange>,
    blocked: bool,
    playing: bool,
    interval: u32,
    due: Option<u64>,
    last_clock: Option<u64>,
}

impl MonitorSet {
    pub fn new() -> Result<Self, Error> {
        let instance = NEXT_INSTANCE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| value.checked_add(1))
            .map_err(|_| Error::Exhausted)?;
        Ok(Self {
            instance,
            serial: 0,
            input: 0,
            exhausted: false,
            sources: Vec::new(),
            // The constructor range is exactly SlotId's five supported values.
            order: std::array::from_fn(|index| SlotId::try_from(index as u8).expect("bounded stable slot")),
            active: None,
            bindings: [None; 5],
            pending: None,
            blocked: false,
            playing: true,
            interval: 10,
            due: None,
            last_clock: None,
        })
    }

    fn fail_closed(&mut self) {
        self.sources.clear();
        self.active = None;
        self.bindings = [None; 5];
        self.pending = None;
        self.playing = false;
        self.due = None;
        self.blocked = true;
    }

    fn allocate(&mut self) -> Result<u64, Error> {
        let Some(serial) = self.serial.checked_add(1).filter(|_| !self.exhausted) else {
            self.exhausted = true;
            self.fail_closed();
            return Err(Error::Exhausted);
        };
        self.serial = serial;
        Ok(serial)
    }

    fn clock(&mut self, now: u64) -> Result<(), Error> {
        if self.last_clock.is_some_and(|previous| now < previous) {
            self.cancel_input()?;
            self.playing = false;
            self.due = None;
            return Err(Error::Clock);
        }
        self.last_clock = Some(now);
        Ok(())
    }

    fn deadline(&mut self, now: u64) -> Result<u64, Error> {
        let Some(due) = now.checked_add(u64::from(self.interval) * 1000) else {
            self.pause()?;
            return Err(Error::Clock);
        };
        Ok(due)
    }

    /// Invalid catalogues fail closed instead of keeping earlier private source
    /// bindings. Ordinary activation alone neither rebinds nor disables a view.
    pub fn sync(&mut self, sources: &[Source], order: [SlotId; MAX_WORKERS], active: Option<PreviewSource>) -> Result<(), Error> {
        let invalid = sources.len() > MAX_WORKERS
            || order.iter().enumerate().any(|(index, slot)| order[..index].contains(slot))
            || sources.iter().enumerate().any(|(index, source)| {
                sources[..index].iter().any(|other| {
                    other.slot == source.slot
                        || other.preview.worker() == source.preview.worker()
                        || other.preview.session() == source.preview.session()
                })
            })
            || active.is_some_and(|active| !sources.iter().any(|source| source.preview == active));
        if invalid || self.exhausted {
            self.fail_closed();
            return Err(if self.exhausted { Error::Exhausted } else { Error::Invalid });
        }
        self.sources = sources.to_vec();
        self.order = order;
        self.active = active;
        for binding in self.bindings.iter_mut().flatten() {
            if !sources.iter().any(|source| *source == binding.source && source.available) {
                // Tombstones retain the user's choice, never a replacement at
                // the same stable slot. Restoration requires explicit Open/Next.
                binding.enabled = false;
            }
        }
        Ok(())
    }

    pub fn binding(&self, view: View) -> Option<Binding> {
        self.bindings[view.index()]
    }

    /// Presentation acknowledgement only. The parent must first correlate the
    /// actual typed settings receipt to its immutable pending request. A later
    /// local focus/Chat change is not a different source binding or a failed ACK.
    pub fn ack_options(&mut self, view: View, expected: Binding, options: Options) -> Result<bool, Error> {
        if self.binding(view) != Some(expected) || !expected.enabled || !self.current(expected.source) {
            return Ok(false);
        }
        if expected.options != options {
            self.cancel_input()?;
            if let Some(binding) = &mut self.bindings[view.index()] {
                binding.options = options;
            }
        }
        Ok(true)
    }

    fn current(&self, source: Source) -> bool {
        !self.exhausted && source.available && self.sources.contains(&source)
    }

    fn fixed_for(&self, source: Source) -> Option<View> {
        View::ALL[..FIXED].iter().copied().find(|view| {
            self.binding(*view)
                .is_some_and(|binding| binding.enabled && binding.source == source)
        })
    }

    fn bind(&mut self, view: View, source: Source, options: Options, visible: bool) -> Result<Binding, Error> {
        if !self.current(source) {
            return Err(Error::Unavailable);
        }
        let binding = Binding {
            source,
            incarnation: self.allocate()?,
            enabled: true,
            visible,
            options,
        };
        self.bindings[view.index()] = Some(binding);
        Ok(binding)
    }

    /// Explicit Open also accepts current Active for remembered-Active sampling;
    /// it never promotes, renders or subscribes any other source.
    pub fn open_fixed(&mut self, source: Source) -> Result<View, Error> {
        if self.blocked || self.pending.is_some() {
            return Err(Error::Busy);
        }
        if !self.current(source) {
            return Err(Error::Unavailable);
        }
        if let Some(view) = self.fixed_for(source) {
            self.set_visible(view, true)?;
            return Ok(view);
        }
        let view = View::ALL[..FIXED]
            .iter()
            .copied()
            .find(|view| self.binding(*view).is_none_or(|binding| !binding.enabled))
            .ok_or(Error::Capacity)?;
        self.bind(view, source, Options::default(), true)?;
        Ok(view)
    }

    pub fn close(&mut self, view: View) -> Result<Option<Binding>, Error> {
        self.cancel_input()?;
        let old = self.bindings[view.index()].take();
        if view == View::Cycling {
            self.playing = false;
            self.due = None;
        }
        // Caller compares visible_sources before/after and clears last-view
        // cache immediately; old IO rate reservations are a separate owner.
        Ok(old)
    }

    pub fn set_visible(&mut self, view: View, visible: bool) -> Result<(), Error> {
        if self.binding(view).is_some_and(|binding| binding.visible != visible) {
            self.cancel_input()?;
            if let Some(binding) = &mut self.bindings[view.index()] {
                binding.visible = visible;
            }
            if view == View::Cycling {
                self.due = None;
            }
        }
        Ok(())
    }

    /// Deduplicated explicit visible bindings, including current Active. These
    /// are not inactive PreviewDemand values and grant no image authority.
    pub fn visible_sources(&self) -> Vec<Source> {
        let mut result = Vec::with_capacity(MAX_WORKERS);
        for binding in self.bindings.iter().flatten() {
            if binding.enabled && binding.visible && self.current(binding.source) && !result.contains(&binding.source) {
                result.push(binding.source);
            }
        }
        result
    }

    pub fn cancel_input(&mut self) -> Result<(), Error> {
        self.input = self.allocate()?;
        Ok(())
    }

    pub fn set_blocked(&mut self, blocked: bool) -> Result<(), Error> {
        if self.blocked != blocked {
            self.cancel_input()?;
            self.blocked = blocked;
            self.due = None;
        }
        Ok(())
    }

    fn origin(&self, view: View) -> Option<Ticket> {
        if self.blocked || self.pending.is_some() || self.exhausted {
            return None;
        }
        let binding = self.binding(view).filter(|binding| binding.visible)?;
        Some(Ticket {
            instance: self.instance,
            input: self.input,
            view,
            binding,
        })
    }

    fn origin_current(&self, ticket: Ticket) -> bool {
        self.origin(ticket.view) == Some(ticket)
    }

    /// Local presentation/cycle scope only, including an unavailable tombstone.
    /// Native/source actions still require validate()/can_exchange().
    pub fn capture_view(&self, view: View) -> Option<Ticket> {
        self.origin(view)
    }

    pub fn validate_view(&self, ticket: Ticket) -> bool {
        self.origin_current(ticket)
    }

    pub fn capture(&self, view: View) -> Option<Ticket> {
        let ticket = self.origin(view)?;
        (ticket.binding.enabled && self.current(ticket.source())).then_some(ticket)
    }

    pub fn validate(&self, ticket: Ticket) -> bool {
        self.capture(ticket.view) == Some(ticket)
    }

    fn next(&self, slot: Option<SlotId>, direction: Direction) -> Option<Source> {
        let step = if direction == Direction::Next { 1 } else { MAX_WORKERS - 1 };
        let mut at = slot
            .and_then(|slot| self.order.iter().position(|item| *item == slot))
            .unwrap_or(if direction == Direction::Next { MAX_WORKERS - 1 } else { 0 });
        for _ in 0..MAX_WORKERS {
            at = (at + step) % MAX_WORKERS;
            if let Some(source) = self
                .sources
                .iter()
                .find(|source| source.slot == self.order[at] && source.available && Some(source.preview) != self.active)
            {
                return Some(*source);
            }
        }
        None
    }

    /// Navigation is local to the cycler and captures the exact target at press.
    /// An unavailable paused assignment can be left only by explicit Next/Play.
    pub fn capture_navigation(&self, direction: Direction) -> Option<Navigation> {
        let origin = self.origin(View::Cycling)?;
        let target = self.next(Some(origin.source().slot), direction)?;
        Some(Navigation { origin, target })
    }

    pub fn validate_navigation(&self, navigation: Navigation) -> bool {
        self.origin_current(navigation.origin) && self.current(navigation.target) && Some(navigation.target.preview) != self.active
    }

    pub fn navigate(&mut self, navigation: Navigation) -> Result<Option<Changed>, Error> {
        if !self.validate_navigation(navigation) {
            return Ok(None);
        }
        self.pause()?;
        self.select_cycle(navigation.target)
    }

    fn select_cycle(&mut self, source: Source) -> Result<Option<Changed>, Error> {
        let old = self.binding(View::Cycling).ok_or(Error::Unavailable)?;
        if old.source == source && old.enabled {
            return Ok(None);
        }
        self.bind(View::Cycling, source, old.options, old.visible)?;
        Ok(Some(Changed {
            view: View::Cycling,
            previous: old.source,
            current: source,
        }))
    }

    pub fn open_cycler(&mut self, now: u64) -> Result<(), Error> {
        self.clock(now)?;
        if self.blocked || self.pending.is_some() {
            return Err(Error::Busy);
        }
        if self.binding(View::Cycling).is_some() {
            return self.set_visible(View::Cycling, true);
        }
        let source = self.next(None, Direction::Next).ok_or(Error::Unavailable)?;
        let due = self.deadline(now)?;
        self.bind(View::Cycling, source, Options::default(), true)?;
        self.playing = true;
        self.due = Some(due);
        Ok(())
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    pub fn interval(&self) -> u32 {
        self.interval
    }

    pub fn pause(&mut self) -> Result<(), Error> {
        self.cancel_input()?;
        self.playing = false;
        self.due = None;
        Ok(())
    }

    pub fn play(&mut self, now: u64) -> Result<Option<Changed>, Error> {
        self.clock(now)?;
        if self.blocked || self.pending.is_some() {
            return Err(Error::Busy);
        }
        let old = self.binding(View::Cycling).ok_or(Error::Unavailable)?;
        let due = self.deadline(now)?;
        self.cancel_input()?;
        let changed = if old.enabled && self.current(old.source) && Some(old.source.preview) != self.active {
            None
        } else {
            let source = self.next(Some(old.source.slot), Direction::Next).ok_or(Error::Unavailable)?;
            self.select_cycle(source)?
        };
        self.playing = true;
        self.due = Some(due);
        Ok(changed)
    }

    pub fn set_interval(&mut self, seconds: u32, now: u64) -> Result<(), Error> {
        self.clock(now)?;
        if !(2..=3600).contains(&seconds) {
            return Err(Error::Invalid);
        }
        self.interval = seconds;
        self.due = if self.playing && self.binding(View::Cycling).is_some() {
            Some(self.deadline(now)?)
        } else {
            None
        };
        Ok(())
    }

    /// Timer output is a local binding transition only. It cannot acknowledge
    /// reads, send/activate, create image demand or change Shared Chat selection.
    pub fn tick(&mut self, now: u64) -> Result<Option<Changed>, Error> {
        self.clock(now)?;
        let Some(binding) = self.binding(View::Cycling) else {
            return Ok(None);
        };
        if !self.playing {
            return Ok(None);
        }
        if self.blocked || self.pending.is_some() || !binding.visible {
            self.due = None;
            return Ok(None);
        }
        let valid = binding.enabled && self.current(binding.source) && Some(binding.source.preview) != self.active;
        if valid {
            let Some(due) = self.due else {
                self.due = Some(self.deadline(now)?);
                return Ok(None);
            };
            if now < due {
                return Ok(None);
            }
        }
        // One transition maximum, never a catch-up loop after a slow frame.
        self.due = Some(self.deadline(now)?);
        let Some(source) = self.next(Some(binding.source.slot), Direction::Next) else {
            if let Some(binding) = &mut self.bindings[View::Cycling.index()] {
                binding.enabled = false;
            }
            return Ok(None);
        };
        self.select_cycle(source)
    }

    pub fn can_exchange(&self, ticket: Ticket) -> bool {
        self.validate(ticket) && Some(ticket.source().preview) != self.active
    }

    pub fn begin_exchange(&mut self, ticket: Ticket) -> Result<Option<Exchange>, Error> {
        if !self.can_exchange(ticket) {
            return Ok(None);
        }
        let previous = self
            .active
            .and_then(|active| self.sources.iter().find(|source| source.preview == active).copied());
        let operation = self.allocate()?;
        self.cancel_input()?; // Consumes this gesture even if the eventual switch rolls back.
        if ticket.view == View::Cycling {
            self.playing = false;
            self.due = None;
        }
        let exchange = Exchange {
            instance: self.instance,
            operation,
            origin: ticket,
            previous,
        };
        self.pending = Some(exchange);
        Ok(Some(exchange))
    }

    /// `success` must come from Shell's exact acknowledged Coordinator operation,
    /// after sync has published its current Active. Queue acceptance is not success.
    pub fn finish_exchange(&mut self, exchange: Exchange, success: bool) -> Result<ExchangeResult, Error> {
        if self.pending != Some(exchange) || exchange.instance != self.instance {
            return Ok(ExchangeResult::Ignored);
        }
        self.pending = None; // Exact outcome is consumed once, including failure.
        let view = exchange.view();
        let Some(binding) = self.binding(view) else {
            return Ok(ExchangeResult::Ignored);
        };
        if binding.incarnation != exchange.origin.binding.incarnation || binding.source != exchange.target() {
            return Ok(ExchangeResult::Ignored);
        }
        if !success {
            return Ok(ExchangeResult::Kept);
        }
        let previous = exchange.previous.filter(|source| self.current(*source));
        if self.active != Some(exchange.target().preview)
            || !binding.enabled
            || !self.current(exchange.target())
            || previous.is_none()
            || (view != View::Cycling && previous.is_some_and(|source| self.fixed_for(source).is_some()))
        {
            if let Some(binding) = &mut self.bindings[view.index()] {
                binding.enabled = false;
            }
            return Ok(ExchangeResult::Stopped);
        }
        self.bind(view, previous.ok_or(Error::Unavailable)?, binding.options, binding.visible)?;
        Ok(ExchangeResult::Exchanged)
    }
}

#[cfg(test)]
mod tests;
