//! Terminal UI surrounding the emulator
//!
//! This is the Ratatui implementation of the interactive TUI.

#![expect(unstable_name_collisions)] // From Itertools::intersperse

mod layout;
mod style;
mod widgets;

use crate::{
    backend::terminal::{
        RatatuiTerminal,
        input::{InputAction, InputEvent, TuiAction},
        tui::{
            layout::LayoutCached,
            style::STYLES,
            widgets::{Scrollbar, ScrollbarState},
        },
    },
    debugger::{Debugger, RunState},
    emu::{
        Address, AddressRange, BcdFlags, Clock, Cpu, Cycles, GameBoy,
        InstructionInfo, MemoryBusReadOnly, instruction::Instruction,
    },
    util::{IntDisplay, PackedBits},
};
use itertools::Itertools;
use ratatui::{
    layout::{Constraint, Layout, Rect},
    prelude::Buffer,
    style::Styled,
    symbols::merge::MergeStrategy,
    text::{Line, Span, Text},
    widgets::{
        Block, BorderType, Borders, List, ListState, StatefulWidget, Widget,
    },
};
use ratatui_textarea::TextArea;
use std::iter;

/// Terminal UI surrounding the emulator
///
/// This handles everything drawn to the terminal *except for* the emulator
/// screen. It also handles input events.
///
/// This struct stores the TUI state that's retained between frames.
#[derive(Default)]
pub struct Tui {
    /// Active element in focus, or `None` for the "default" view
    focus: Option<Focus>,
    /// Retained TUI state
    state: TuiWidgetState,
}

impl Tui {
    /// Get the screen area that the emulator screen should be drawn to
    ///
    /// This area is expanded as much as possible, but will be shrunk as
    /// necessary to fit the debugger around it.
    pub fn emulator_area(&self) -> Rect {
        self.state.emulator_area
    }

    /// Draw the TUI to the terminal
    pub fn draw(
        &mut self,
        terminal: &mut RatatuiTerminal,
        emulator: &GameBoy,
        debugger: &Debugger,
    ) {
        terminal
            .draw(|frame| {
                frame.render_stateful_widget(
                    TuiWidget {
                        focus: self.focus.as_ref(),
                        emulator,
                        debugger,
                    },
                    frame.area(),
                    &mut self.state,
                );
            })
            .unwrap();
    }

    /// Update UI state according to an input event
    ///
    /// Return `true` if the event was consumed, `false` if it can be
    /// propagated.
    pub fn update(
        &mut self,
        emulator: &mut GameBoy,
        debugger: &mut Debugger,
        event: InputEvent,
    ) -> bool {
        // Extract the TUI-bound action - it may be useful later
        let action = if let Some(InputAction::Tui(action)) = event.action {
            Some(action)
        } else {
            None
        };
        match &mut self.focus {
            Some(Focus::GoToAddress { text_area }) => match action {
                // Esc/Enter get out of the dialog
                Some(TuiAction::Cancel) => {
                    self.unfocus();
                    true
                }
                Some(TuiAction::Submit) => {
                    if let Ok(address) = text_area.lines()[0].parse::<Address>()
                    {
                        self.state.memory.select_address(address);
                        self.unfocus();
                        true
                    } else {
                        false
                    }
                }
                // Pass everything else to the text box
                _ => text_area.input(event.event),
            },

            None => {
                let Some(action) = action else {
                    return false; // We don't care about this action
                };
                // If it has a bound TUI action, consume it
                match action {
                    TuiAction::Up => {
                        self.state.memory.move_address(Direction::Up);
                    }
                    TuiAction::Down => {
                        self.state.memory.move_address(Direction::Down);
                    }
                    TuiAction::Left => {
                        self.state.memory.move_address(Direction::Left);
                    }
                    TuiAction::Right => {
                        self.state.memory.move_address(Direction::Right);
                    }
                    TuiAction::DebugGoToAddress => {
                        self.focus(Focus::go_to_address());
                    }
                    TuiAction::DebugPauseToggle => debugger.toggle_pause(),
                    TuiAction::DebugStepCycle => debugger.step_cycle(emulator),
                    TuiAction::DebugStepFrame => debugger.step_frame(emulator),
                    TuiAction::DebugStepInstruction => {
                        debugger.step_instruction(emulator);
                    }
                    TuiAction::DebugSnapshotPrevious => {
                        debugger.previous_snapshot(emulator);
                    }
                    TuiAction::DebugSnapshotNext => {
                        debugger.next_snapshot(emulator);
                    }
                    TuiAction::Cancel | TuiAction::Submit => {}
                }
                true
            }
        }
    }

    /// Enter a specific focus mode
    fn focus(&mut self, focus: Focus) {
        self.focus = Some(focus);
    }

    /// Return to unfocused mode
    fn unfocus(&mut self) {
        self.focus = None;
    }
}

/// Active element in focus
enum Focus {
    /// Go To Address text box is focused
    GoToAddress { text_area: TextArea<'static> },
}

impl Focus {
    /// Enter [Self::GoToAddress]
    fn go_to_address() -> Self {
        Self::GoToAddress {
            text_area: TextArea::default(),
        }
    }
}

/// Widget for all interactive elements
struct TuiWidget<'a> {
    focus: Option<&'a Focus>,
    emulator: &'a GameBoy,
    debugger: &'a Debugger,
}

impl StatefulWidget for TuiWidget<'_> {
    type State = TuiWidgetState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // Show breakpoints always so we can set them while running
        let [left_area, memory_area] =
            Layout::horizontal([Constraint::Min(0), 31.into()])
                .areas_cached(area);
        // Leave space for the screen in the top-left
        let [emulator_area, mut bottom_left_area] =
            Layout::vertical([Constraint::Min(0), 20.into()])
                .areas_cached(left_area);
        state.emulator_area = emulator_area; // Store this for the parent
        bottom_left_area.width += 1; // Combine borders into the Memory panel
        // Move down below the screen area
        let [debugger_area, cpu_area] =
            Layout::horizontal([Constraint::Min(0), 36.into()])
                .spacing(-1)
                .areas_cached(bottom_left_area);
        DebuggerPanel {
            emulator: self.emulator,
            debugger: self.debugger,
        }
        .render(debugger_area, buf);

        // Only show emulator state info if the debugger is paused. The state
        // changes too quickly while running to be useful.
        if self.debugger.run_state().is_debugging() {
            let cpu = self.emulator.cpu();
            CpuPanel {
                clock: self.emulator.clock(),
                cpu,
            }
            .render(cpu_area, buf);
            let pc = cpu.registers().pc().0;
            MemoryPanel {
                pc: AddressRange::new(
                    pc,
                    // Bound is INCLUSIVE
                    pc + cpu.current_instruction().size - 1,
                ),
                memory_bus: &self.emulator.memory(),
                go_to_address: if let Some(Focus::GoToAddress { text_area }) =
                    &self.focus
                {
                    Some(text_area)
                } else {
                    None
                },
            }
            .render(memory_area, buf, &mut state.memory);
        }
    }
}

/// Widget state for [TuiWidget]
///
/// This is the state that's retained across calls.
#[derive(Default)]
struct TuiWidgetState {
    /// Area that the emulator screen should be drawn to
    ///
    /// This is calculated dynamically based on the other screen content. It's
    /// retained so it can be reported back up to where the screen is rendered.
    emulator_area: Rect,
    memory: MemoryDetailState,
}

/// Widget for debugger info
struct DebuggerPanel<'a> {
    emulator: &'a GameBoy,
    debugger: &'a Debugger,
}

impl Widget for DebuggerPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = panel("Debugger", area, buf);

        let snapshots = self.debugger.snapshots();
        let breakpoints = self.debugger.breakpoints();
        let [
            state_area,
            snapshots_header_area,
            snapshots_list_area,
            breakpoints_header_area,
            breakpoints_list_area,
        ] = Layout::vertical([
            1,
            1,
            snapshots.len() as u16,
            1,
            breakpoints.len() as u16,
        ])
        .areas_cached(area);

        let state = match self.debugger.run_state() {
            RunState::Paused => "PAUSED",
            RunState::Stepping => "STEPPING",
            RunState::Running => "RUNNING",
        };
        state.render(state_area, buf);

        // Snapshots
        "Snapshots"
            .set_style(STYLES.subheader)
            .render(snapshots_header_area, buf);
        // We don't need an explicit select state. The current emulator *is*
        // the select state. Whichever snapshot matches that is the selected
        // snapshot.
        let list =
            List::new(snapshots.iter().map(|snap| snap.id().to_string()))
                .highlight_symbol(">");
        let selected_snapshot_index = snapshots
            .iter()
            .position(|snap| snap.matches(self.emulator));
        StatefulWidget::render(
            list,
            snapshots_list_area,
            buf,
            &mut ListState::default().with_selected(selected_snapshot_index),
        );

        // Breakpoints
        "Breakpoints"
            .set_style(STYLES.subheader)
            .render(breakpoints_header_area, buf);
        Widget::render(
            List::new(breakpoints.map(|bp| bp.to_string())),
            breakpoints_list_area,
            buf,
        );
    }
}

/// Widget for CPU info
struct CpuPanel<'a> {
    clock: &'a Clock,
    cpu: &'a Cpu,
}

impl Widget for CpuPanel<'_> {
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
struct MemoryPanel<'a> {
    /// Range of bytes defining the next CPU instruction
    pc: AddressRange,
    memory_bus: &'a MemoryBusReadOnly<'a>,
    /// Text box for jumping to an address
    go_to_address: Option<&'a TextArea<'static>>,
}

impl MemoryPanel<'_> {
    /// Bytes shown on each line of the view
    const BYTES_PER_LINE: u16 = 8;
}

impl StatefulWidget for MemoryPanel<'_> {
    type State = MemoryDetailState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let area = panel("Memory", area, buf);
        let [bytes_area, detail_area] =
            Layout::vertical([Constraint::Min(0), 3.into()]).areas_cached(area);

        // Main content - the bytes!!
        MemoryBytes {
            pc: self.pc,
            memory_bus: self.memory_bus,
        }
        .render(bytes_area, buf, state);

        MemoryDetail {
            address: state.selected,
            memory_bus: self.memory_bus,
        }
        .render(detail_area, buf);

        // Go To textbox overlays on the bottom line
        let bottom_area = Rect {
            x: area.x,
            y: area.bottom(),
            width: area.width,
            height: 1,
        };
        // Render the Go To box at the bottom
        if let Some(go_to_address) = self.go_to_address {
            go_to_address.render(bottom_area, buf);
        } else {
            "[g] Go To Address".render(bottom_area, buf);
        }
    }
}

/// Detailed metadata info about the selected byte
struct MemoryDetail<'a> {
    address: Address,
    memory_bus: &'a MemoryBusReadOnly<'a>,
}

impl Widget for MemoryDetail<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let metadata = self.memory_bus.get_metadata(self.address);
        format!(
            "{address}: {block_name}",
            address = metadata.address,
            block_name = metadata.block_name
        )
        .render(area, buf);
    }
}

/// Widget state for [MemoryDetail]
///
/// This is the state that's retained across calls.
struct MemoryDetailState {
    /// Highlighted
    selected: Address,
    /// Vertical scroll state
    ///
    /// This is related to, but not strictly attached to, the selected address.
    /// The selection can move up and down without scrolling. Scrolling occurs
    /// only when the selection would move out of view.
    scroll: ScrollbarState,
}

impl MemoryDetailState {
    /// Update the selection state to jump to a specific memory address
    fn select_address(&mut self, address: Address) {
        self.selected = address;
        // Make sure the selected byte stays in view
        self.scroll
            .scroll_to((address.0 / MemoryPanel::BYTES_PER_LINE).into());
    }

    /// Move the address selection one cell in the given direction
    fn move_address(&mut self, direction: Direction) {
        let offset = match direction {
            Direction::Up => -(MemoryPanel::BYTES_PER_LINE as i16),
            Direction::Down => MemoryPanel::BYTES_PER_LINE as i16,
            Direction::Left => -1,
            Direction::Right => 1,
        };
        // Stop hard at the top/bottom
        if let Some(address) = self.selected.0.checked_add_signed(offset) {
            self.select_address(Address(address));
        }
    }
}

impl Default for MemoryDetailState {
    fn default() -> Self {
        Self {
            selected: Address::default(),
            scroll: ScrollbarState::new(AddressRange::ALL.len()),
        }
    }
}

/// Widget for the byte rows of [MemoryDetail]
struct MemoryBytes<'a> {
    /// Range of bytes defining the next CPU instruction
    pc: AddressRange,
    memory_bus: &'a MemoryBusReadOnly<'a>,
}

impl StatefulWidget for MemoryBytes<'_> {
    type State = MemoryDetailState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        // Split the area vertically once. We know the two will have the same
        // amount of rows, so we can iterate over them together
        let [gutter_area, bytes_area] =
            Layout::horizontal([4.into(), Constraint::Min(0)])
                .spacing(1)
                .areas_cached(area);

        // Draw scrollbar
        Scrollbar::default().render(area, buf, &mut state.scroll);

        // Find the visible lines and iterate over them shits
        let offset = state.scroll.offset() as u16;
        for ((line, gutter_area), bytes_area) in (offset
            ..(offset + area.height))
            .zip(gutter_area.rows())
            .zip(bytes_area.rows())
        {
            let address = Address(line * MemoryPanel::BYTES_PER_LINE);

            // Draw address in gutter
            Span::styled(
                IntDisplay::hex(address.0).without_prefix().to_string(),
                STYLES.memory_gutter,
            )
            .render(gutter_area, buf);

            // Draw each byte as a separate widget. This makes it easy to define
            // per-byte popups
            let byte_areas: [Rect; MemoryPanel::BYTES_PER_LINE as usize] =
                Layout::horizontal(iter::repeat_n(2, 8))
                    .spacing(1)
                    .areas_cached(bytes_area);
            for (address_offset, area) in
                (0..MemoryPanel::BYTES_PER_LINE).zip(byte_areas)
            {
                // Address range size is divisible by BYTES_PER_LINE, so if
                // the start of the line is valid, the entire line will be
                let address = address + address_offset;
                let value = self.memory_bus.get8(address);
                MemoryByte {
                    value,
                    pc: self.pc.contains(address),
                    selected: address == state.selected,
                }
                .render(area, buf);
            }
        }
    }
}

/// Widget for a single byte in the memory view
struct MemoryByte {
    value: u8,
    /// Is this byte part of the current instruction?
    pc: bool,
    /// Is this byte highlighted under the cursor?
    selected: bool,
}

impl Widget for MemoryByte {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut style = STYLES.u8(self.value);
        // Apply extra styles
        if self.pc {
            style = style.patch(STYLES.memory_pc);
        }
        if self.selected {
            style = style.patch(STYLES.memory_selected);
        }
        Span::styled(
            IntDisplay::hex(self.value).without_prefix().to_string(),
            style,
        )
        .render(area, buf);
    }
}

/// Cardinal direction
enum Direction {
    Up,
    Down,
    Left,
    Right,
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
