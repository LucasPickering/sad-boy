//! Terminal UI surrounding the emulator
//!
//! This is the Ratatui implementation of the interactive TUI.

#![expect(unstable_name_collisions)] // From Itertools::intersperse

mod style;

use crate::{
    backend::terminal::{
        RatatuiTerminal, TERM_HEIGHT, TERM_WIDTH, input::TuiEvent,
        tui::style::STYLES,
    },
    debugger::{Debugger, RunState},
    emu::{
        Address, AddressRange, BcdFlags, Clock, Cpu, Cycles, GameBoy,
        InstructionInfo, MemoryBusReadOnly, instruction::Instruction, memory,
    },
    util::{IntDisplay, PackedBits},
};
use itertools::Itertools;
use ratatui::{
    layout::{Constraint, Layout, Margin, Rect},
    prelude::Buffer,
    style::Styled,
    symbols::merge::MergeStrategy,
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, Scrollbar, ScrollbarOrientation,
        ScrollbarState, StatefulWidget, Widget,
    },
};
use std::cell::RefCell;

/// Terminal UI surrounding the emulator
///
/// This handles everything drawn to the terminal *except for* the emulator
/// screen. It also handles input events.
///
/// This struct stores the TUI state that's retained between frames.
pub struct Tui {
    /// Vertical scroll state for the Memory panel
    memory_scroll: ScrollbarState,
}

impl Tui {
    /// Draw the TUI to the terminal
    pub fn draw(
        &mut self,
        terminal: &mut RatatuiTerminal,
        emulator: &GameBoy,
        debugger: &Debugger,
    ) {
        terminal
            .draw(|frame| {
                frame.render_widget(
                    TuiWidget {
                        emulator,
                        debugger,
                        memory_scroll: &mut self.memory_scroll,
                    },
                    frame.area(),
                );
            })
            .unwrap();
    }

    /// Update UI state according to an input event
    pub fn update(
        &mut self,
        emulator: &GameBoy,
        debugger: &mut Debugger,
        event: TuiEvent,
    ) {
        match event {
            TuiEvent::Up => self.memory_scroll.prev(),
            TuiEvent::Down => self.memory_scroll.next(),
            TuiEvent::Left => {}
            TuiEvent::Right => {}
            TuiEvent::DebugPauseToggle => debugger.toggle_pause(),
            TuiEvent::DebugStepCycle => debugger.step_cycle(emulator),
            TuiEvent::DebugStepFrame => debugger.step_frame(emulator),
            TuiEvent::DebugStepInstruction => {
                debugger.step_instruction(emulator);
            }
        }
    }
}

impl Default for Tui {
    fn default() -> Self {
        Self {
            memory_scroll: ScrollbarState::new(AddressRange::ALL.len()),
        }
    }
}

/// Widget for all interactive elements
struct TuiWidget<'a> {
    emulator: &'a GameBoy,
    debugger: &'a Debugger,
    /// Vertical scroll state for the Memory panel
    memory_scroll: &'a mut ScrollbarState,
}

impl Widget for TuiWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Show breakpoints always so we can set them while running
        let areas = TuiAreas::get(area);

        DebuggerInfo(self.debugger).render(areas.debug, buf);

        // Only show emulator state info if the debugger is paused. The state
        // changes too quickly while running to be useful.
        if self.debugger.run_state().should_show_debugger() {
            let cpu = self.emulator.cpu();
            CpuInfo {
                clock: self.emulator.clock(),
                cpu,
            }
            .render(areas.cpu, buf);
            let pc = cpu.registers().pc().0;
            MemoryInfo {
                pc: AddressRange::new(
                    pc,
                    // Bound is INCLUSIVE
                    pc + cpu.current_instruction().size - 1,
                ),
                scroll: self.memory_scroll,
                memory_bus: self.emulator.memory(),
            }
            .render(areas.memory, buf);
        }
    }
}

/// Split areas for the main TUI layout
#[derive(Clone, Copy)]
struct TuiAreas {
    debug: Rect,
    cpu: Rect,
    memory: Rect,
}

impl TuiAreas {
    /// Split the given area for the TUI layout
    fn get(area: Rect) -> Self {
        // The area splitting is expensive (~40% of the frame time) but it
        // only changes if the terminal is resized. Caching it speeds up rapid
        // stepping by a lot.
        thread_local! {
            static CACHE: RefCell<Option<(Rect, TuiAreas)>> =
                const { RefCell::new(None) };
        }

        // Check the cache
        if let Some(cached) = CACHE.with(|cache| {
            cache
                .borrow()
                .filter(|(key, _)| *key == area)
                .map(|(_, value)| value)
        }) {
            return cached;
        }

        // Show breakpoints always so we can set them while running
        let [left, memory, _] = Layout::horizontal([
            TERM_WIDTH.into(),
            31.into(),
            Constraint::Min(0),
        ])
        .areas(area);
        // Leave space for the screen in the top-left
        let [_, mut bottom_left] =
            Layout::vertical([TERM_HEIGHT.into(), Constraint::Min(0)])
                .areas(left);
        bottom_left.width += 1; // Combine borders into the Memory panel
        // Move down below the screen area
        let [debug, cpu] = Layout::horizontal([Constraint::Min(0), 36.into()])
            .spacing(-1)
            .areas(bottom_left);
        let areas = Self { debug, cpu, memory };

        // Cache this layout
        CACHE.with(|cache| *cache.borrow_mut() = Some((area, areas)));
        areas
    }
}

/// Widget for debugger info
struct DebuggerInfo<'a>(&'a Debugger);

impl Widget for DebuggerInfo<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = panel("Debugger", area, buf);
        let mut text = Text::from_iter([
            match self.0.run_state() {
                RunState::Paused => "PAUSED",
                RunState::Stepping => "STEPPING",
                RunState::Running => "RUNNING",
            }
            .into(),
            "Breakpoints".set_style(STYLES.subheader),
        ]);
        for breakpoint in self.0.breakpoints() {
            text.push_line(breakpoint.to_string());
        }
        text.render(area, buf);
    }
}

/// Widget for CPU info
struct CpuInfo<'a> {
    clock: &'a Clock,
    cpu: &'a Cpu,
}

impl Widget for CpuInfo<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        fn fmt_reg8(name: &'static str, value: u8) -> Line<'static> {
            Line::from_iter([
                format!("{name}: ").into(),
                Span::styled(
                    format!(
                        "{value} ({hex}, {bin})",
                        hex = IntDisplay::hex(value),
                        bin = IntDisplay::binary(value),
                    ),
                    STYLES.u8(value),
                ),
            ])
        }

        fn fmt_reg16(name: &'static str, value: u16) -> Line<'static> {
            Line::from_iter([
                format!("{name}: ").into(),
                Span::styled(
                    format!("{value} ({hex})", hex = IntDisplay::hex(value)),
                    STYLES.u16(value),
                ),
            ])
        }

        fn fmt_reg_addr(name: &'static str, value: Address) -> Line<'static> {
            format!("{name}: {value}").into()
        }

        fn fmt_reg_flags(
            name: &'static str,
            value: PackedBits<BcdFlags>,
        ) -> Line<'static> {
            fn flag(name: char, value: bool) -> Span<'static> {
                Span::styled(
                    format!(
                        "{name}={value}",
                        value = if value { '1' } else { '0' }
                    ),
                    STYLES.bool(value),
                )
            }

            let flags = value.unpack();
            [
                format!("{name}:").into(),
                IntDisplay::hex(value.as_u8()).to_string().into(),
                flag('z', flags.zero),
                flag('n', flags.subtract),
                flag('h', flags.half_carry),
                flag('c', flags.carry),
            ]
            .into_iter()
            .intersperse(" ".into())
            .collect()
        }

        let area = panel("CPU", area, buf);

        let previous =
            self.cpu.previous_instruction().unwrap_or(InstructionInfo {
                instruction: Instruction::Invalid,
                duration: Cycles(0),
                end: Cycles(0),
                size: 0,
            });
        let next = self.cpu.current_instruction();
        let registers = self.cpu.registers();
        let lines: [Line; _] = [
            format!("CLOCK: {}", self.clock.cycles()).into(),
            format!(
                "PREV: {instr} ({dur}cy/{size}B)",
                instr = previous.instruction,
                dur = previous.duration,
                size = previous.size,
            )
            .into(),
            format!(
                "NEXT: {instr} ({dur}cy/{size}B)",
                instr = next.instruction,
                dur = next.duration,
                size = next.size,
            )
            .into(),
            // Registers
            fmt_reg_addr("pc", registers.pc()),
            fmt_reg_addr("sp", registers.sp()),
            fmt_reg8("a", registers.a()),
            fmt_reg_flags("f", registers.f()),
            fmt_reg16("af", registers.af()),
            fmt_reg8("b", registers.b()),
            fmt_reg8("c", registers.c()),
            fmt_reg16("bc", registers.bc()),
            fmt_reg8("d", registers.d()),
            fmt_reg8("e", registers.e()),
            fmt_reg16("de", registers.de()),
            fmt_reg8("h", registers.h()),
            fmt_reg8("l", registers.l()),
            fmt_reg16("hl", registers.hl()),
            format!(
                "INT: {}",
                if self.cpu.interrupts_enabled() {
                    "ENABLE"
                } else {
                    "DISABLE"
                }
            )
            .into(),
        ];
        Text::from_iter(lines).render(area, buf);
    }
}

/// Widget to inspect memory
struct MemoryInfo<'a> {
    /// Range of bytes defining the next CPU instruction
    pc: AddressRange,
    /// Vertical scroll state offset
    scroll: &'a mut ScrollbarState,
    memory_bus: MemoryBusReadOnly<'a>,
}

impl MemoryInfo<'_> {
    /// Get all memory ranges that should be labelled
    fn labelled_ranges(&self) -> impl Iterator<Item = AddressRange> {
        const PRELUDE_BOOTSTRAP: &[AddressRange] = &[
            memory::BOOTSTRAP,
            AddressRange::named(
                "Cartridge ROM",
                memory::BOOTSTRAP.last().0 + 1,
                memory::CARTRIDGE_ROM.last().0,
            ),
        ];
        const PRELUDE_NORMAL: &[AddressRange] = &[memory::CARTRIDGE_ROM];
        const REST: &[AddressRange] = &[
            memory::TILE_DATA,
            memory::TILE_MAPS,
            memory::CARTRIDGE_RAM,
            memory::RAM,
            memory::ECHO_RAM,
            memory::OAM,
            memory::HIGH_RAM,
        ];
        // While the bootstrap ROM is mounted, it overlays the cartridge ROM
        let prelude = if self.memory_bus.is_bootstrapping() {
            PRELUDE_BOOTSTRAP
        } else {
            PRELUDE_NORMAL
        };
        prelude.iter().chain(REST.iter()).copied()
    }
}

impl Widget for MemoryInfo<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        const BYTES_PER_LINE: u16 = 8;

        // Format a memory address in the gutter
        fn fmt_address(address: Address) -> Span<'static> {
            Span::styled(
                IntDisplay::hex(address.0).without_prefix().to_string(),
                STYLES.memory_gutter,
            )
        }

        // Format a single byte to text
        let fmt_byte = |address: Address, value: u8| -> Span {
            let mut style = STYLES.u8(value);
            if self.pc.contains(address) {
                style = style.patch(STYLES.memory_pc);
            }
            Span::styled(
                IntDisplay::hex(value).without_prefix().to_string(),
                style,
            )
        };

        let area = panel("Memory", area, buf);

        // Build up the text until we fill the area. The while loop is easier
        // than an iterator because the number of lines is dynamic based on
        // what labels are visible
        let mut text = Text::default();
        let mut next_address =
            Address(self.scroll.get_position() as u16 * BYTES_PER_LINE);
        while text.height() < area.height.into() {
            // Add a label at the start of each region
            if let Some(labelled_range) = self
                .labelled_ranges()
                .find(|range| range.start() == next_address)
            {
                text.push_line(Span::styled(
                    labelled_range.name().unwrap(),
                    STYLES.memory_range_label,
                ));
            }

            // Address range size is divisible by BYTES_PER_LINE, so if
            // the start of the line is valid, the entire line will be
            let bytes = (0..BYTES_PER_LINE).map(|offset| {
                let address = next_address + offset;
                let value = self.memory_bus.get8(address);
                fmt_byte(address, value)
            });

            let line = [fmt_address(next_address)]
                .into_iter()
                .chain(bytes)
                .intersperse(" ".into())
                .collect::<Line>();
            text.push_line(line);

            if let Some(next) = next_address.checked_add(BYTES_PER_LINE) {
                next_address = next;
            } else {
                // Hit the end of the address range
                break;
            }
        }
        text.render(area, buf);

        // Draw scrollbar
        Scrollbar::new(ScrollbarOrientation::VerticalRight).render(
            area.outer(Margin::new(1, 0)), // Overlay the panel border
            buf,
            self.scroll,
        );
    }
}

/// Draw an outline for a panel, returning the inner area
fn panel(title: &'_ str, area: Rect, buf: &mut Buffer) -> Rect {
    let block = Block::new()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .merge_borders(MergeStrategy::Fuzzy);
    (&block).render(area, buf);
    block.inner(area)
}
