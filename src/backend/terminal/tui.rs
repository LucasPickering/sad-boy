//! Terminal UI surrounding the debugger
//!
//! This is the Ratatui implementation of the interactive TUI.

use crate::{
    backend::terminal::{TERM_HEIGHT, TERM_WIDTH},
    debugger::Debugger,
    emu::{
        Clock, Cpu, Cycles, GameBoy, InstructionInfo, instruction::Instruction,
    },
    util::IntDisplay,
};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    prelude::Buffer,
    style::{Modifier, Styled},
    symbols::merge::MergeStrategy,
    text::Text,
    widgets::{Block, BorderType, Borders, Widget},
};
use std::fmt::{self, Display};

/// Widget to draw debug info to the terminal
pub struct DebugInfo<'a> {
    pub emulator: &'a GameBoy,
    pub debugger: &'a Debugger,
}

impl Widget for DebugInfo<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
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
        CpuInfo {
            clock: self.emulator.clock(),
            cpu: self.emulator.cpu(),
        }
        .render(cpu_area, buf);

        // Memory
        panel("Memory", memory_area, buf);
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
        /// Register display helper
        struct Reg<T>(T);

        impl Display for Reg<u8> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{v} ({hex}, {bin})",
                    v = self.0,
                    hex = IntDisplay::hex(self.0),
                    bin = IntDisplay::binary(self.0),
                )
            }
        }

        impl Display for Reg<u16> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(
                    f,
                    "{v} ({hex})",
                    v = self.0,
                    hex = IntDisplay::hex(self.0),
                )
            }
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
        let lines = [
            format!("CLOCK: {}", self.clock.cycles()),
            format!(
                "PREV: {instr} ({dur}cy/{size}B)",
                instr = previous.instruction,
                dur = previous.duration,
                size = previous.size,
            ),
            format!(
                "NEXT: {instr} ({dur}cy/{size}B)",
                instr = next.instruction,
                dur = next.duration,
                size = next.size,
            ),
            // Registers
            format!("pc: {}", registers.pc()),
            format!("sp: {}", registers.sp()),
            format!("a: {}", Reg(registers.a())),
            format!(
                "f: {} {}",
                IntDisplay::hex(registers.f().as_u8()),
                registers.f().unpack()
            ),
            format!("af: {}", Reg(registers.af())),
            format!("b: {}", Reg(registers.b())),
            format!("c: {}", Reg(registers.c())),
            format!("bc: {}", Reg(registers.bc())),
            format!("d: {}", Reg(registers.d())),
            format!("e: {}", Reg(registers.e())),
            format!("de: {}", Reg(registers.de())),
            format!("h: {}", Reg(registers.h())),
            format!("l: {}", Reg(registers.l())),
            format!("hl: {}", Reg(registers.hl())),
            format!(
                "INT: {}",
                if self.cpu.interrupts_enabled() {
                    "ENABLE"
                } else {
                    "DISABLE"
                }
            ),
        ];
        Text::from_iter(lines).render(area, buf);
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
