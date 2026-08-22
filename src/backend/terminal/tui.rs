//! Terminal UI surrounding the emulator
//!
//! This is the Ratatui implementation of the interactive TUI.

use crate::{
    backend::terminal::{
        RatatuiTerminal, TERM_HEIGHT, TERM_WIDTH, input::TuiEvent,
    },
    debugger::Debugger,
    emu::{
        Address, AddressRange, Clock, Cpu, Cycles, GameBoy, InstructionInfo,
        MemoryBusReadOnly, instruction::Instruction,
    },
    util::IntDisplay,
};
use itertools::Itertools;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    prelude::Buffer,
    style::{Color, Modifier, Style, Styled},
    symbols::merge::MergeStrategy,
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Widget},
};

/// Terminal UI surrounding the emulator
///
/// This handles everything drawn to the terminal *except for* the emulator
/// screen. It also handles input events.
#[derive(Default)]
pub struct Tui {}

impl Tui {
    /// Draw the TUI to the terminal
    pub fn draw(
        &self,
        terminal: &mut RatatuiTerminal,
        emulator: &GameBoy,
        debugger: &Debugger,
    ) {
        terminal
            .draw(|frame| {
                frame.render_widget(
                    TuiWidget { emulator, debugger },
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
            TuiEvent::DebugPauseToggle => debugger.toggle_pause(),
            TuiEvent::DebugStepCycle => debugger.step_cycle(emulator),
            TuiEvent::DebugStepFrame => debugger.step_frame(emulator),
            TuiEvent::DebugStepInstruction => {
                debugger.step_instruction(emulator);
            }
        }
    }
}

/// Widget for all interactive elements
struct TuiWidget<'a> {
    emulator: &'a GameBoy,
    debugger: &'a Debugger,
}

impl Widget for TuiWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Show breakpoints always so we can set them while running
        let [left_area, memory_area] =
            Layout::horizontal([TERM_WIDTH.into(), Constraint::Min(0)])
                .areas(area);
        // Leave space for the screen in the top-left
        let [_, mut bottom_left_area] =
            Layout::vertical([TERM_HEIGHT.into(), Constraint::Min(0)])
                .areas(left_area);
        bottom_left_area.width += 1; // Combine borders into the Memory panel
        // Move down below the screen area
        let [debugger_area, cpu_area] =
            Layout::horizontal([Constraint::Min(0), 36.into()])
                .spacing(-1)
                .areas(bottom_left_area);

        DebuggerInfo(self.debugger).render(debugger_area, buf);

        // Only show emulator state info if the debugger is paused. The state
        // changes too quickly while running to be useful.
        if self.debugger.paused() {
            let cpu = self.emulator.cpu();
            CpuInfo {
                clock: self.emulator.clock(),
                cpu,
            }
            .render(cpu_area, buf);
            let pc = cpu.registers().pc().0;
            MemoryInfo {
                offset: Address(0x0000),
                pc: AddressRange::new(
                    pc,
                    // Bound is INCLUSIVE
                    pc + cpu.current_instruction().size - 1,
                ),
                memory_bus: self.emulator.memory(),
            }
            .render(memory_area, buf);
        }
    }
}

/// Widget for debugger info
struct DebuggerInfo<'a>(&'a Debugger);

impl Widget for DebuggerInfo<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = panel("Debugger", area, buf);
        let mut text = Text::from_iter([
            if self.0.paused() { "PAUSED" } else { "RUNNING" }.into(),
            "Breakpoints".set_style(Modifier::UNDERLINED),
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
                    u8_style(value),
                ),
            ])
        }

        fn fmt_reg16(name: &'static str, value: u16) -> Line<'static> {
            Line::from_iter([
                format!("{name}: ").into(),
                Span::styled(
                    format!("{value} ({hex})", hex = IntDisplay::hex(value)),
                    u16_style(value),
                ),
            ])
        }

        fn fmt_address(name: &'static str, value: Address) -> Line<'static> {
            format!("{name}: {value}").into()
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
            fmt_address("pc", registers.pc()),
            fmt_address("sp", registers.sp()),
            fmt_reg8("a", registers.a()),
            format!(
                "f: {} {}",
                IntDisplay::hex(registers.f().as_u8()),
                registers.f().unpack()
            )
            .into(),
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
    /// First visible address
    offset: Address,
    /// Range of bytes defining the next CPU instruction
    pc: AddressRange,
    memory_bus: MemoryBusReadOnly<'a>,
}

impl Widget for MemoryInfo<'_> {
    #[expect(unstable_name_collisions)] // From Itertools::intersperse
    fn render(self, area: Rect, buf: &mut Buffer) {
        const BYTES_PER_LINE: u16 = 8;

        // Format a memory address in the gutter
        let fmt_address = |address: Address| -> Span {
            Span::styled(
                IntDisplay::hex(address.0).without_prefix().to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            )
        };

        // Format a single byte to text
        let fmt_byte = |address: Address, value: u8| -> Span {
            let modifier = if self.pc.contains(address) {
                Modifier::UNDERLINED
            } else {
                Modifier::empty()
            };
            Span::styled(
                IntDisplay::hex(value).without_prefix().to_string(),
                u8_style(value).add_modifier(modifier),
            )
        };

        let area = panel("Memory", area, buf);
        let text: Text = (0..area.height)
            // Cap at 0xffff
            .filter_map(|y| self.offset.checked_add(y * BYTES_PER_LINE))
            .map(|address| {
                // Address range size is divisible by BYTES_PER_LINE, so if
                // the start of the line is valid, the entire line will be
                let bytes = (0..BYTES_PER_LINE).map(|offset| {
                    let address = address + offset;
                    let value = self.memory_bus.get8(address);
                    fmt_byte(address, value)
                });

                [fmt_address(address)]
                    .into_iter()
                    .chain(bytes)
                    .intersperse(" ".into())
                    .collect::<Line>()
            })
            .collect();
        text.render(area, buf);
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

/// Get text styling for an 8-bit value
///
/// This provides some visual guidance when reading bytes.
/// https://simonomi.dev/blog/color-code-your-bytes/
fn u8_style(value: u8) -> Style {
    // https://github.com/simonomi/hexapoda/blob/bf8bd6297d649b3fb1f100bdc99272705fa558b3/src/buffer/widget/hex.rs#L210
    let color = match value {
        0x00 => Color::Rgb(0x80, 0x80, 0x80), // grey
        0x01..0x10 => Color::Rgb(0xFF, 0x71, 0xA9), // red
        0x10..0x20 => Color::Rgb(0xFF, 0x7A, 0x78), // salmon
        0x20..0x30 => Color::Rgb(0xFF, 0x81, 0x23), // red-orange
        0x30..0x40 => Color::Rgb(0xF7, 0x93, 0x00), // yellow-orange
        0x40..0x50 => Color::Rgb(0xE6, 0x9F, 0x00), // yellow
        0x50..0x60 => Color::Rgb(0xC1, 0xB2, 0x00), // green-yellow
        0x60..0x70 => Color::Rgb(0x82, 0xC6, 0x00), // lime
        0x70..0x80 => Color::Rgb(0x00, 0xD5, 0x00), // green
        0x80..0x90 => Color::Rgb(0x00, 0xD4, 0x59), // clover
        0x90..0xA0 => Color::Rgb(0x00, 0xD0, 0x91), // teal
        0xA0..0xB0 => Color::Rgb(0x00, 0xCC, 0xBB), // cyan
        0xB0..0xC0 => Color::Rgb(0x00, 0xC7, 0xDE), // light blue
        0xC0..0xD0 => Color::Rgb(0x00, 0xBE, 0xFF), // blue
        0xD0..0xE0 => Color::Rgb(0x6C, 0xAF, 0xFF), // blurple
        0xE0..0xF0 => Color::Rgb(0xB2, 0x98, 0xFF), // purple
        0xF0..0xFF => Color::Rgb(0xFF, 0x4D, 0xFF), // pink
        0xFF => Color::White,
    };
    Style::new().fg(color)
}

/// Get text styling for a 16-bit value
///
/// This provides some visual guidance when reading bytes.
/// https://simonomi.dev/blog/color-code-your-bytes/
fn u16_style(value: u16) -> Style {
    u8_style(value.to_be_bytes()[0])
}
