mod action_types;
mod clipboard_utils;
mod keybind_utils;
mod line;
mod tab;
mod tooltip;

use std::cmp::{max, min};
use std::collections::{BTreeMap, HashMap};
use std::convert::TryInto;
use std::path::PathBuf;

use tab::get_tab_to_focus;
use zellij_tile::prelude::*;

use crate::clipboard_utils::{system_clipboard_error, text_copied_hint};
use crate::line::tab_line;
use crate::tab::{render_pane_name, tab_style};
use crate::tooltip::TooltipRenderer;

static ARROW_SEPARATOR: &str = "";

const CONFIG_IS_TOOLTIP: &str = "is_tooltip";
const CONFIG_TOGGLE_TOOLTIP_KEY: &str = "tooltip";
const CONFIG_TAB_FORMAT: &str = "tab_format";
const DEFAULT_TAB_FORMAT: &str = "{index}. {name}";
const CONFIG_PANE_FORMAT: &str = "pane_format";
const MSG_TOGGLE_TOOLTIP: &str = "toggle_tooltip";
const MSG_TOGGLE_PERSISTED_TOOLTIP: &str = "toggle_persisted_tooltip";
const MSG_LAUNCH_TOOLTIP: &str = "launch_tooltip_if_not_launched";

#[derive(Debug, Default)]
pub struct LinePart {
    part: String,
    len: usize,
    tab_index: Option<usize>,
}

#[derive(Default)]
struct State {
    // Tab state
    tabs: Vec<TabInfo>,
    active_tab_idx: usize,
    pane_manifest: PaneManifest,
    process_names: HashMap<usize, String>,
    cwd_names: HashMap<usize, String>,
    git_repo_status: HashMap<usize, bool>,
    own_initial_cwd: PathBuf,
    pane_format: String,
    last_pane_names: HashMap<u32, String>,
    original_pane_names: HashMap<u32, String>,

    // Display state
    mode_info: ModeInfo,
    tab_line: Vec<LinePart>,
    breadcrumb_range: Option<(usize, usize)>,
    display_area_rows: usize,
    display_area_cols: usize,

    // Clipboard state
    text_copy_destination: Option<CopyDestination>,
    display_system_clipboard_failure: bool,

    // Plugin configuration
    config: BTreeMap<String, String>,
    own_plugin_id: Option<u32>,
    toggle_tooltip_key: Option<String>,
    tab_format: String,

    // Tooltip state
    is_tooltip: bool,
    tooltip_is_active: bool,
    persist: bool,
    is_first_run: bool,
    own_tab_index: Option<usize>,
    own_client_id: u16,

    // Keybinding cache
    cached_keybinds: KeybindsVec,
}

struct TabRenderData {
    tabs: Vec<LinePart>,
    active_tab_index: usize,
    active_swap_layout_name: Option<String>,
    is_swap_layout_dirty: bool,
}

register_plugin!(State);

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        let plugin_ids = get_plugin_ids();
        self.own_plugin_id = Some(plugin_ids.plugin_id);
        self.own_client_id = plugin_ids.client_id;
        self.own_initial_cwd = plugin_ids.initial_cwd;
        self.initialize_configuration(configuration);
        self.setup_subscriptions();
        self.configure_keybinds();
    }

    fn update(&mut self, event: Event) -> bool {
        self.is_first_run = false;

        match event {
            Event::InitialKeybinds(keybinds) => {
                self.cached_keybinds = keybinds;
                if !self.cached_keybinds.is_empty() {
                    self.mode_info.keybinds = self.cached_keybinds.clone();
                }
                true
            },
            Event::ModeUpdate(mut mode_info) => {
                if mode_info.keybinds.is_empty() && !self.cached_keybinds.is_empty() {
                    mode_info.keybinds = self.cached_keybinds.clone();
                } else if !mode_info.keybinds.is_empty() {
                    self.cached_keybinds = mode_info.keybinds.clone();
                }
                self.handle_mode_update(mode_info)
            },
            Event::TabUpdate(tabs) => self.handle_tab_update(tabs),
            Event::PaneUpdate(pane_manifest) => self.handle_pane_update(pane_manifest),
            Event::Mouse(mouse_event) => {
                self.handle_mouse_event(mouse_event);
                false
            },
            Event::CopyToClipboard(copy_destination) => {
                self.handle_clipboard_copy(copy_destination)
            },
            Event::SystemClipboardFailure => self.handle_clipboard_failure(),
            Event::InputReceived => self.handle_input_received(),
            Event::CommandChanged(pane_id, command, _, _) => {
                self.handle_command_changed(pane_id, command)
            },
            Event::CwdChanged(pane_id, cwd, _) => self.handle_cwd_changed(pane_id, cwd),
            Event::PermissionRequestResult(PermissionStatus::Granted) => {
                set_selectable(false);
                self.resubscribe_events();
                true
            },
            Event::PermissionRequestResult(PermissionStatus::Denied) => {
                eprintln!("Permission denied - compact bar will not function properly");
                false
            },
            _ => false,
        }
    }

    fn pipe(&mut self, message: PipeMessage) -> bool {
        if self.is_tooltip && message.is_private {
            self.handle_tooltip_pipe(message);
        } else if message.name == MSG_TOGGLE_TOOLTIP
            && message.is_private
            && self.toggle_tooltip_key.is_some()
            // only launch once per plugin instance
            && self.own_tab_index == Some(self.active_tab_idx.saturating_sub(1))
            // only launch once per client of plugin instance
            && Some(format!("{}", self.own_client_id)) == message.payload
        {
            self.toggle_persisted_tooltip(self.mode_info.mode);
        }
        false
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if self.is_tooltip {
            self.render_tooltip(rows, cols);
        } else {
            self.resolve_missing_process_data();
            self.render_tab_line(cols);
        }
    }
}

impl State {
    fn initialize_configuration(&mut self, configuration: BTreeMap<String, String>) {
        self.config = configuration.clone();
        self.is_tooltip = self.parse_bool_config(CONFIG_IS_TOOLTIP, false);
        self.tab_format = configuration
            .get(CONFIG_TAB_FORMAT)
            .cloned()
            .unwrap_or_else(|| DEFAULT_TAB_FORMAT.to_string());
        self.pane_format = configuration
            .get(CONFIG_PANE_FORMAT)
            .cloned()
            .unwrap_or_default();

        if !self.is_tooltip {
            if let Some(tooltip_toggle_key) = configuration.get(CONFIG_TOGGLE_TOOLTIP_KEY) {
                self.toggle_tooltip_key = Some(tooltip_toggle_key.clone());
            }
        }

        if self.is_tooltip {
            self.is_first_run = true;
        }
    }

    fn setup_subscriptions(&self) {
        if self.is_tooltip {
            set_selectable(false);

            subscribe(&[
                EventType::ModeUpdate,
                EventType::TabUpdate,
                EventType::InitialKeybinds,
            ]);
        } else {
            request_permission(&[
                PermissionType::ReadApplicationState,
                PermissionType::ChangeApplicationState,
            ]);

            let mut events = vec![
                EventType::TabUpdate,
                EventType::PaneUpdate,
                EventType::ModeUpdate,
                EventType::Mouse,
                EventType::CopyToClipboard,
                EventType::InputReceived,
                EventType::SystemClipboardFailure,
                EventType::PermissionRequestResult,
                EventType::InitialKeybinds,
            ];
            let (needs_process, needs_smart, needs_cwd) = self.placeholder_requirements();
            if needs_process || needs_smart {
                events.push(EventType::CommandChanged);
            }
            if needs_smart || needs_cwd {
                events.push(EventType::CwdChanged);
            }
            subscribe(&events);
        }
    }

    fn resubscribe_events(&self) {
        let mut events = vec![
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::ModeUpdate,
            EventType::Mouse,
            EventType::CopyToClipboard,
            EventType::InputReceived,
            EventType::SystemClipboardFailure,
            EventType::InitialKeybinds,
        ];
        let (needs_process, needs_smart, needs_cwd) = self.placeholder_requirements();
        if needs_process || needs_smart {
            events.push(EventType::CommandChanged);
        }
        if needs_smart || needs_cwd {
            events.push(EventType::CwdChanged);
        }
        subscribe(&events);
    }

    fn placeholder_requirements(&self) -> (bool, bool, bool) {
        let format_text = format!("{} {}", self.tab_format, self.pane_format);
        (
            format_text.contains("{process}"),
            format_text.contains("{smart}"),
            format_text.contains("{cwd}"),
        )
    }

    fn configure_keybinds(&self) {
        if !self.is_tooltip && self.toggle_tooltip_key.is_some() {
            if let Some(toggle_key) = &self.toggle_tooltip_key {
                reconfigure(
                    bind_toggle_key_config(toggle_key, self.own_client_id),
                    false,
                );
            }
        }
    }

    fn parse_bool_config(&self, key: &str, default: bool) -> bool {
        self.config
            .get(key)
            .and_then(|v| v.parse().ok())
            .unwrap_or(default)
    }

    // Event handlers
    fn handle_mode_update(&mut self, mode_info: ModeInfo) -> bool {
        let should_render = self.mode_info != mode_info;
        let old_mode = self.mode_info.mode;
        let new_mode = mode_info.mode;
        let base_mode = mode_info.base_mode.unwrap_or(InputMode::Normal);

        self.mode_info = mode_info;

        if self.is_tooltip {
            self.handle_tooltip_mode_update(old_mode, new_mode, base_mode);
        } else {
            self.handle_main_mode_update(new_mode, base_mode);
        }

        should_render
    }

    fn handle_main_mode_update(&self, new_mode: InputMode, base_mode: InputMode) {
        if self.toggle_tooltip_key.is_some()
            && new_mode != base_mode
            && !self.is_restricted_mode(new_mode)
        {
            self.launch_tooltip_if_not_launched(new_mode);
        }
    }

    fn handle_tooltip_mode_update(
        &mut self,
        old_mode: InputMode,
        new_mode: InputMode,
        base_mode: InputMode,
    ) {
        if !self.persist && (new_mode == base_mode || self.is_restricted_mode(new_mode)) {
            close_self();
        } else if new_mode != old_mode || self.persist {
            self.update_tooltip_for_mode_change(new_mode);
        }
    }

    fn handle_tab_update(&mut self, tabs: Vec<TabInfo>) -> bool {
        self.update_display_area(&tabs);

        if let Some(active_tab_index) = tabs.iter().position(|t| t.active) {
            let active_tab_idx = active_tab_index + 1; // Convert to 1-based indexing
            let should_render = self.active_tab_idx != active_tab_idx || self.tabs != tabs;

            if self.is_tooltip && self.active_tab_idx != active_tab_idx {
                self.move_tooltip_to_new_tab(active_tab_idx);
            }

            self.active_tab_idx = active_tab_idx;
            self.tabs = tabs;
            should_render
        } else {
            false
        }
    }

    fn handle_pane_update(&mut self, pane_manifest: PaneManifest) -> bool {
        let manifest_changed = self.pane_manifest != pane_manifest;
        self.pane_manifest = pane_manifest;

        if self.toggle_tooltip_key.is_some() {
            let previous_tooltip_state = self.tooltip_is_active;
            self.tooltip_is_active = self.detect_tooltip_presence(&self.pane_manifest);
            self.own_tab_index = self.find_own_tab_index(&self.pane_manifest);
            manifest_changed || previous_tooltip_state != self.tooltip_is_active
        } else {
            manifest_changed
        }
    }

    fn handle_mouse_event(&mut self, mouse_event: Mouse) {
        if self.is_tooltip {
            return;
        }

        match mouse_event {
            Mouse::LeftClick(_, col) => {
                if self.click_is_on_breadcrumb(col) {
                    focus_host_session();
                } else {
                    self.handle_tab_click(col);
                }
            },
            Mouse::ScrollUp(_) => self.scroll_tab_up(),
            Mouse::ScrollDown(_) => self.scroll_tab_down(),
            _ => {},
        }
    }

    fn click_is_on_breadcrumb(&self, col: usize) -> bool {
        matches!(self.breadcrumb_range, Some((start, end)) if col >= start && col < end)
    }

    fn handle_clipboard_copy(&mut self, copy_destination: CopyDestination) -> bool {
        if self.is_tooltip {
            return false;
        }

        let should_render = match self.text_copy_destination {
            Some(current) => current != copy_destination,
            None => true,
        };

        self.text_copy_destination = Some(copy_destination);
        should_render
    }

    fn handle_clipboard_failure(&mut self) -> bool {
        if self.is_tooltip {
            return false;
        }

        self.display_system_clipboard_failure = true;
        true
    }

    fn handle_input_received(&mut self) -> bool {
        if self.is_tooltip {
            return false;
        }

        let should_render =
            self.text_copy_destination.is_some() || self.display_system_clipboard_failure;
        self.clear_clipboard_state();
        should_render
    }

    fn handle_tooltip_pipe(&mut self, message: PipeMessage) {
        if message.name == MSG_TOGGLE_PERSISTED_TOOLTIP {
            if self.is_first_run {
                self.persist = true;
            } else {
                #[cfg(target_family = "wasm")]
                close_self();
            }
        }
    }

    // Helper methods
    fn update_display_area(&mut self, tabs: &[TabInfo]) {
        for tab in tabs {
            if tab.active {
                self.display_area_rows = tab.display_area_rows;
                self.display_area_cols = tab.display_area_columns;
                break;
            }
        }
    }

    fn detect_tooltip_presence(&self, pane_manifest: &PaneManifest) -> bool {
        for (_tab_index, panes) in &pane_manifest.panes {
            for pane in panes {
                if pane.plugin_url == Some("zellij:compact-bar".to_owned())
                    && pane.pane_x != pane.pane_content_x
                {
                    return true;
                }
            }
        }
        false
    }

    fn find_own_tab_index(&self, pane_manifest: &PaneManifest) -> Option<usize> {
        for (tab_index, panes) in &pane_manifest.panes {
            for pane in panes {
                if pane.is_plugin && Some(pane.id) == self.own_plugin_id {
                    return Some(*tab_index);
                }
            }
        }
        None
    }

    fn handle_tab_click(&self, col: usize) {
        if let Some(tab_idx) = get_tab_to_focus(&self.tab_line, self.active_tab_idx, col) {
            switch_tab_to(tab_idx.try_into().unwrap());
        }
    }

    fn scroll_tab_up(&self) {
        let next_tab = min(self.active_tab_idx + 1, self.tabs.len());
        switch_tab_to(next_tab as u32);
    }

    fn scroll_tab_down(&self) {
        let prev_tab = max(self.active_tab_idx.saturating_sub(1), 1);
        switch_tab_to(prev_tab as u32);
    }

    fn clear_clipboard_state(&mut self) {
        self.text_copy_destination = None;
        self.display_system_clipboard_failure = false;
    }

    fn is_restricted_mode(&self, mode: InputMode) -> bool {
        matches!(
            mode,
            InputMode::Locked
                | InputMode::EnterSearch
                | InputMode::RenameTab
                | InputMode::RenamePane
                | InputMode::Prompt
                | InputMode::Tmux
        )
    }

    // Tooltip operations
    fn toggle_persisted_tooltip(&self, new_mode: InputMode) {
        #[allow(unused_variables)]
        let message = self
            .create_tooltip_message(MSG_TOGGLE_PERSISTED_TOOLTIP, new_mode)
            .with_args(self.create_persist_args());

        #[cfg(target_family = "wasm")]
        pipe_message_to_plugin(message);
    }

    fn launch_tooltip_if_not_launched(&self, new_mode: InputMode) {
        let message = self.create_tooltip_message(MSG_LAUNCH_TOOLTIP, new_mode);
        pipe_message_to_plugin(message);
    }

    fn create_tooltip_message(&self, name: &str, mode: InputMode) -> MessageToPlugin {
        let mut tooltip_config = self.config.clone();
        tooltip_config.insert(CONFIG_IS_TOOLTIP.to_string(), "true".to_string());

        MessageToPlugin::new(name)
            .with_plugin_url("zellij:OWN_URL")
            .with_plugin_config(tooltip_config)
            .with_floating_pane_coordinates(self.calculate_tooltip_coordinates())
            .new_plugin_instance_should_have_pane_title(format!("{:?}", mode))
    }

    fn create_persist_args(&self) -> BTreeMap<String, String> {
        let mut args = BTreeMap::new();
        args.insert("persist".to_string(), String::new());
        args
    }

    fn update_tooltip_for_mode_change(&self, new_mode: InputMode) {
        if let Some(plugin_id) = self.own_plugin_id {
            let coordinates = self.calculate_tooltip_coordinates();
            change_floating_panes_coordinates(vec![(PaneId::Plugin(plugin_id), coordinates)]);
            rename_plugin_pane(plugin_id, format!("{:?}", new_mode));
        }
    }

    fn move_tooltip_to_new_tab(&self, new_tab_index: usize) {
        if let Some(plugin_id) = self.own_plugin_id {
            break_panes_to_tab_with_index(
                &[PaneId::Plugin(plugin_id)],
                new_tab_index.saturating_sub(1), // Convert to 0-based indexing
                false,
            );
        }
    }

    fn calculate_tooltip_coordinates(&self) -> FloatingPaneCoordinates {
        let tooltip_renderer = TooltipRenderer::new(&self.mode_info);
        let (tooltip_rows, tooltip_cols) =
            tooltip_renderer.calculate_dimensions(self.mode_info.mode);

        let width = tooltip_cols + 4; // 2 for borders, 2 for padding
        let height = tooltip_rows + 2; // 2 for borders
        let x_position = 2;
        let y_position = self.display_area_rows.saturating_sub(height + 2);

        FloatingPaneCoordinates::new(
            Some(x_position.to_string()),
            Some(y_position.to_string()),
            Some(width.to_string()),
            Some(height.to_string()),
            Some(true),
            Some(false),
        )
        .unwrap_or_default()
    }

    // Rendering
    fn render_tooltip(&self, rows: usize, cols: usize) {
        let tooltip_renderer = TooltipRenderer::new(&self.mode_info);
        tooltip_renderer.render(rows, cols);
    }

    fn render_tab_line(&mut self, cols: usize) {
        if let Some(copy_destination) = self.text_copy_destination {
            self.render_clipboard_hint(copy_destination);
        } else if self.display_system_clipboard_failure {
            self.render_clipboard_error();
        } else {
            self.render_tabs(cols);
        }
    }

    fn render_clipboard_hint(&self, copy_destination: CopyDestination) {
        let hint = text_copied_hint(copy_destination).part;
        self.render_background_with_text(&hint);
    }

    fn render_clipboard_error(&self) {
        let hint = system_clipboard_error().part;
        self.render_background_with_text(&hint);
    }

    fn render_background_with_text(&self, text: &str) {
        let background = self.mode_info.style.colors.text_unselected.background;
        match background {
            PaletteColor::Rgb((r, g, b)) => {
                print!("{}\u{1b}[48;2;{};{};{}m\u{1b}[0K", text, r, g, b);
            },
            PaletteColor::EightBit(color) => {
                print!("{}\u{1b}[48;5;{}m\u{1b}[0K", text, color);
            },
        }
    }

    fn render_tabs(&mut self, cols: usize) {
        if self.tabs.is_empty() {
            return;
        }

        let tab_data = self.prepare_tab_data();
        let tab_line_output = tab_line(
            &self.mode_info,
            tab_data,
            cols,
            self.toggle_tooltip_key.clone(),
            self.tooltip_is_active,
        );
        self.tab_line = tab_line_output.parts;
        self.breadcrumb_range = tab_line_output.breadcrumb_range;

        let output = self
            .tab_line
            .iter()
            .fold(String::new(), |acc, part| acc + &part.part);

        self.render_background_with_text(&output);
    }

    fn prepare_tab_data(&self) -> TabRenderData {
        let mut all_tabs = Vec::new();
        let mut active_tab_index = 0;
        let mut active_swap_layout_name = None;
        let mut is_swap_layout_dirty = false;
        let mut is_alternate_tab = false;
        let dimmed = self.mode_info.session_ascended == Some(true)
            || self.mode_info.session_dimmed == Some(true);

        for tab in &self.tabs {
            let tab_name = self.get_tab_display_name(tab);
            let process_name = self.get_raw_process_name_for_tab(tab);
            let smart_name = self.get_smart_name_for_tab(tab);
            let cwd_name = self.get_cwd_name_for_tab(tab);

            if tab.active {
                active_tab_index = tab.position;
                if self.mode_info.mode != InputMode::RenameTab {
                    is_swap_layout_dirty = tab.is_swap_layout_dirty;
                    active_swap_layout_name = tab.active_swap_layout_name.clone();
                }
            }

            let format = if is_default_name(&tab.name, "Tab #") {
                &self.tab_format
            } else {
                "{name}"
            };

            let styled_tab = tab_style(
                tab_name,
                tab,
                is_alternate_tab,
                self.mode_info.style.colors,
                self.mode_info.capabilities,
                dimmed,
                format,
                &process_name,
                &smart_name,
                &cwd_name,
            );

            is_alternate_tab = !is_alternate_tab;
            all_tabs.push(styled_tab);
        }

        TabRenderData {
            tabs: all_tabs,
            active_tab_index,
            active_swap_layout_name,
            is_swap_layout_dirty,
        }
    }

    fn get_tab_display_name(&self, tab: &TabInfo) -> String {
        let mut tab_name = tab.name.clone();
        if tab.active && self.mode_info.mode == InputMode::RenameTab && tab_name.is_empty() {
            tab_name = "Enter name...".to_string();
        }
        tab_name
    }

    fn get_smart_name_for_tab(&self, tab: &TabInfo) -> String {
        let process = self.process_names.get(&tab.position);
        let cwd = self.cwd_names.get(&tab.position);
        match (process, cwd) {
            // in a git repo, always show the folder (even while running a process)
            (_, Some(cwd)) if self.git_repo_status.get(&tab.position) == Some(&true) => cwd.clone(),
            // in a shell, show the folder
            (Some(process), Some(cwd)) if is_shell(process) => cwd.clone(),
            // otherwise show the process name
            (Some(process), _) => process.clone(),
            _ => tab.name.clone(),
        }
    }

    fn get_raw_process_name_for_tab(&self, tab: &TabInfo) -> String {
        self.process_names
            .get(&tab.position)
            .cloned()
            .unwrap_or_else(|| tab.name.clone())
    }

    fn resolve_missing_process_data(&mut self) {
        let (needs_process, needs_smart, needs_cwd) = self.placeholder_requirements();
        let needs_command = needs_process || needs_smart;
        let cwd_is_needed = needs_smart || needs_cwd;
        if !needs_command && !cwd_is_needed && self.pane_format.is_empty() {
            return;
        }
        for tab in &self.tabs {
            let position = tab.position;
            let has_process = !needs_command || self.process_names.contains_key(&position);
            let has_cwd = !cwd_is_needed || self.cwd_names.contains_key(&position);
            let has_git_status = !needs_smart || self.git_repo_status.contains_key(&position);
            if has_process && has_cwd && has_git_status {
                continue;
            }
            let Some(pane_id) = self.active_terminal_pane_id_for_tab(position) else {
                continue;
            };
            if !has_process {
                if let Ok(command) = get_pane_running_command(pane_id) {
                    if let Some(process_name) = command.first().map(|c| process_name_from_command(c)) {
                        self.process_names.insert(position, process_name);
                    }
                }
            }
            if !has_cwd || !has_git_status {
                if let Ok(cwd) = get_pane_cwd(pane_id) {
                    if !has_cwd {
                        let folder_name = folder_name_from_cwd(&cwd);
                        self.cwd_names.insert(position, folder_name);
                    }
                    if !has_git_status {
                        let in_git = self.is_in_git_repo(&cwd);
                        self.git_repo_status.insert(position, in_git);
                    }
                }
            }
        }
        if !self.pane_format.is_empty() {
            self.apply_pane_format();
        }
    }

    fn apply_pane_format(&mut self) {
        if self.pane_format.is_empty() || self.is_tooltip {
            return;
        }
        let tabs = self.tabs.clone();
        let format = self.pane_format.clone();
        for tab in &tabs {
            let position = tab.position;
            let Some(pane_id) = self.active_terminal_pane_id_for_tab(position) else {
                continue;
            };
            let PaneId::Terminal(terminal_id) = pane_id else {
                continue;
            };
            if self.pane_has_manual_name(terminal_id) {
                continue;
            }
            let name = self.render_pane_name_for_tab(&format, tab, terminal_id);
            if self.last_pane_names.get(&terminal_id) != Some(&name) {
                rename_terminal_pane(terminal_id, &name);
                self.last_pane_names.insert(terminal_id, name);
            }
        }
    }

    fn render_pane_name_for_tab(
        &mut self,
        format: &str,
        tab: &TabInfo,
        terminal_id: u32,
    ) -> String {
        let process = self.get_raw_process_name_for_tab(tab);
        let smart = self.get_smart_name_for_tab(tab);
        let cwd = self.get_cwd_name_for_tab(tab);
        let name = self.original_pane_name(terminal_id);
        let index = self.pane_index_in_tab(terminal_id);
        render_pane_name(index, &name, &process, &smart, &cwd, "", format)
    }

    fn pane_has_manual_name(&self, terminal_id: u32) -> bool {
        let Some(title) = self.pane_title_for_id(terminal_id) else {
            return false;
        };
        if self.last_pane_names.get(&terminal_id).map(|n| n == &title).unwrap_or(false) {
            return false;
        }
        !is_default_name(&title, "Pane #")
    }

    fn original_pane_name(&mut self, terminal_id: u32) -> String {
        if let Some(name) = self.original_pane_names.get(&terminal_id) {
            return name.clone();
        }
        let name = self
            .pane_title_for_id(terminal_id)
            .unwrap_or_default();
        if !name.is_empty() {
            self.original_pane_names.insert(terminal_id, name.clone());
        }
        name
    }

    fn pane_title_for_id(&self, terminal_id: u32) -> Option<String> {
        for panes in self.pane_manifest.panes.values() {
            if let Some(pane) = panes.iter().find(|p| !p.is_plugin && p.id == terminal_id) {
                return Some(pane.title.clone());
            }
        }
        None
    }

    fn pane_index_in_tab(&self, terminal_id: u32) -> usize {
        for panes in self.pane_manifest.panes.values() {
            if let Some(index) = panes.iter().position(|p| !p.is_plugin && p.id == terminal_id) {
                return index + 1;
            }
        }
        0
    }

    fn active_terminal_pane_id_for_tab(&self, tab_position: usize) -> Option<PaneId> {
        let panes = self.pane_manifest.panes.get(&tab_position)?;
        let pane = panes
            .iter()
            .find(|p| !p.is_plugin && !p.is_suppressed && p.is_focused)
            .or_else(|| panes.iter().find(|p| !p.is_plugin && !p.is_suppressed))?;
        Some(PaneId::Terminal(pane.id))
    }

    fn get_cwd_name_for_tab(&self, tab: &TabInfo) -> String {
        self.cwd_names
            .get(&tab.position)
            .cloned()
            .unwrap_or_else(|| tab.name.clone())
    }

    fn handle_cwd_changed(&mut self, pane_id: PaneId, cwd: PathBuf) -> bool {
        let folder_name = folder_name_from_cwd(&cwd);
        let Some(tab_position) = self.find_tab_position_for_pane(pane_id) else {
            return false;
        };
        let in_git = self.is_in_git_repo(&cwd);
        let folder_changed = self
            .cwd_names
            .get(&tab_position)
            .map_or(true, |n| n != &folder_name);
        let git_changed = self.git_repo_status.get(&tab_position) != Some(&in_git);
        if folder_changed || git_changed {
            self.cwd_names.insert(tab_position, folder_name);
            self.git_repo_status.insert(tab_position, in_git);
        }
        folder_changed || git_changed
    }

    fn is_in_git_repo(&self, cwd: &std::path::Path) -> bool {
        let Ok(relative_cwd) = cwd.strip_prefix(&self.own_initial_cwd) else {
            return false;
        };
        let mut dir = std::path::PathBuf::from("/host").join(relative_cwd);
        loop {
            if std::fs::metadata(dir.join(".git")).is_ok() {
                return true;
            }
            if !dir.pop() {
                break;
            }
        }
        false
    }

    fn handle_command_changed(&mut self, pane_id: PaneId, command: Vec<String>) -> bool {
        let Some(process_name) = command.first().map(|c| process_name_from_command(c)) else {
            return false;
        };
        let Some(tab_position) = self.find_tab_position_for_pane(pane_id) else {
            return false;
        };
        if self.process_names.get(&tab_position).map_or(true, |n| n != &process_name) {
            self.process_names.insert(tab_position, process_name);
            true
        } else {
            false
        }
    }

    fn find_tab_position_for_pane(&self, pane_id: PaneId) -> Option<usize> {
        for (tab_position, panes) in &self.pane_manifest.panes {
            let matching_id = match pane_id {
                PaneId::Terminal(id) => id,
                PaneId::Plugin(_) => continue,
            };
            if panes.iter().any(|p| !p.is_plugin && p.id == matching_id) {
                return Some(*tab_position);
            }
        }
        None
    }
}

fn bind_toggle_key_config(toggle_key: &str, client_id: u16) -> String {
    format!(
        r#"
        keybinds {{
            shared {{
                bind "{}" {{
                  MessagePlugin "compact-bar" {{
                      name "toggle_tooltip"
                      tooltip "{}"
                      payload "{}"
                  }}
                }}
            }}
        }}
    "#,
        toggle_key, toggle_key, client_id
    )
}

fn process_name_from_command(command: &str) -> String {
    let name = std::path::Path::new(command)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| command.to_string());
    name.strip_prefix('-').unwrap_or(&name).to_string()
}

fn is_default_name(name: &str, prefix: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
}

fn is_shell(process_name: &str) -> bool {
    matches!(
        process_name,
        "bash"
            | "zsh"
            | "fish"
            | "sh"
            | "dash"
            | "ksh"
            | "mksh"
            | "ash"
            | "csh"
            | "tcsh"
            | "nu"
            | "elvish"
            | "xonsh"
            | "pwsh"
            | "powershell"
    )
}

fn folder_name_from_cwd(cwd: &std::path::Path) -> String {
    cwd.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cwd.to_string_lossy().to_string())
}
