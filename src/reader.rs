use crate::parser::StyledLine;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    style::{Color, Print, SetForegroundColor, ResetColor, Attribute, SetAttribute},
    terminal::{self, ClearType},
};
use std::io::{stdout, Write};

/// A visual line is a single row on the terminal screen.
/// We pre-wrap all logical lines into visual lines so scrolling
/// works correctly regardless of paragraph length.
struct VisualLine {
    text: String,
}

pub struct Reader {
    visual_lines: Vec<VisualLine>,
    scroll: usize,
    chapter_title: String,
    chapter_index: usize,
    total_chapters: usize,
}

pub enum ReaderAction {
    Quit,
    NextChapter,
    PrevChapter,
    SelectChapter,
}

/// Measure the visible (printed) width of a string, ignoring ANSI escape sequences.
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c == 'm' {
                in_escape = false;
            }
            continue;
        }
        len += 1;
    }
    len
}

/// Split a string with ANSI codes into chunks that each fit within `max_width`
/// visible characters. Preserves ANSI codes across splits so styling continues.
fn wrap_ansi_line(line: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 || line.is_empty() {
        return vec![line.to_string()];
    }

    let mut result: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_visible = 0;
    // Track active ANSI codes so we can re-apply them on the next line
    let mut active_codes: Vec<String> = Vec::new();

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\x1b' {
            // Capture entire ANSI escape sequence
            let mut seq = String::new();
            seq.push(chars[i]);
            i += 1;
            while i < chars.len() {
                seq.push(chars[i]);
                if chars[i] == 'm' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            // Track resets and new codes
            if seq.contains("[0m") || seq.contains("[m") {
                active_codes.clear();
            } else {
                active_codes.push(seq.clone());
            }
            current.push_str(&seq);
        } else {
            if current_visible >= max_width {
                // Close any active styling before line break
                if !active_codes.is_empty() {
                    current.push_str("\x1b[0m");
                }
                result.push(current);
                // Start new line and re-apply active codes
                current = String::new();
                for code in &active_codes {
                    current.push_str(code);
                }
                current_visible = 0;
            }
            current.push(chars[i]);
            current_visible += 1;
            i += 1;
        }
    }

    if !current.is_empty() || result.is_empty() {
        result.push(current);
    }

    result
}

/// Convert logical styled lines into visual lines that each fit one terminal row.
fn build_visual_lines(lines: &[StyledLine], term_width: usize) -> Vec<VisualLine> {
    let mut visual = Vec::new();
    for line in lines {
        if line.text.is_empty() || visible_len(&line.text) == 0 {
            visual.push(VisualLine { text: String::new() });
        } else {
            for wrapped in wrap_ansi_line(&line.text, term_width) {
                visual.push(VisualLine { text: wrapped });
            }
        }
    }
    visual
}

impl Reader {
    pub fn new(
        lines: Vec<StyledLine>,
        chapter_title: &str,
        chapter_index: usize,
        total_chapters: usize,
    ) -> Self {
        // We'll build visual lines on first render (need terminal width)
        Self {
            visual_lines: Vec::new(),
            scroll: 0,
            chapter_title: chapter_title.to_string(),
            chapter_index,
            total_chapters,
        }
        .with_visual_lines(lines)
    }

    fn with_visual_lines(mut self, lines: Vec<StyledLine>) -> Self {
        let width = terminal::size().map(|(c, _)| c as usize).unwrap_or(80);
        self.visual_lines = build_visual_lines(&lines, width.saturating_sub(1));
        self
    }

    pub fn run(&mut self) -> anyhow::Result<ReaderAction> {
        terminal::enable_raw_mode()?;
        let mut stdout = stdout();
        execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

        let result = self.event_loop();

        execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
        terminal::disable_raw_mode()?;

        result
    }

    fn event_loop(&mut self) -> anyhow::Result<ReaderAction> {
        self.render()?;

        loop {
            if let Event::Key(key) = event::read()? {
                match (key.code, key.modifiers) {
                    (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                        return Ok(ReaderAction::Quit);
                    }
                    (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                        return Ok(ReaderAction::Quit);
                    }
                    (KeyCode::Down, _) | (KeyCode::Char('j'), _) => {
                        self.scroll_down(1);
                    }
                    (KeyCode::Up, _) | (KeyCode::Char('k'), _) => {
                        self.scroll_up(1);
                    }
                    (KeyCode::PageDown, _) | (KeyCode::Char(' '), _) | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                        let (_, rows) = terminal::size()?;
                        self.scroll_down((rows as usize).saturating_sub(3));
                    }
                    (KeyCode::PageUp, _) | (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                        let (_, rows) = terminal::size()?;
                        self.scroll_up((rows as usize).saturating_sub(3));
                    }
                    (KeyCode::Home, _) | (KeyCode::Char('g'), _) => {
                        self.scroll = 0;
                    }
                    (KeyCode::End, _) | (KeyCode::Char('G'), _) => {
                        let (_, rows) = terminal::size()?;
                        let content_rows = (rows as usize).saturating_sub(2);
                        self.scroll = self.visual_lines.len().saturating_sub(content_rows);
                    }
                    (KeyCode::Char('n'), _) => {
                        return Ok(ReaderAction::NextChapter);
                    }
                    (KeyCode::Char('p'), _) => {
                        return Ok(ReaderAction::PrevChapter);
                    }
                    (KeyCode::Char('t'), _) => {
                        return Ok(ReaderAction::SelectChapter);
                    }
                    _ => {}
                }
                self.render()?;
            }
        }
    }

    fn scroll_down(&mut self, amount: usize) {
        let (_, rows) = terminal::size().unwrap_or((80, 24));
        let content_rows = (rows as usize).saturating_sub(2);
        let max = self.visual_lines.len().saturating_sub(content_rows);
        self.scroll = (self.scroll + amount).min(max);
    }

    fn scroll_up(&mut self, amount: usize) {
        self.scroll = self.scroll.saturating_sub(amount);
    }

    fn render(&self) -> anyhow::Result<()> {
        let mut stdout = stdout();
        let (cols, rows) = terminal::size()?;
        let content_rows = (rows as usize).saturating_sub(2); // 1 header + 1 footer

        execute!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;

        // Header bar
        let header = format!(
            " {} ({}/{})",
            self.chapter_title,
            self.chapter_index + 1,
            self.total_chapters
        );
        let header_padded = format!("{:<width$}", header, width = cols as usize);
        execute!(
            stdout,
            SetForegroundColor(Color::Black),
            crossterm::style::SetBackgroundColor(Color::Cyan),
            Print(&header_padded),
            ResetColor,
            Print("\r\n")
        )?;

        // Content - now using visual lines that each fit one terminal row
        let end = (self.scroll + content_rows).min(self.visual_lines.len());
        for i in self.scroll..end {
            let line = &self.visual_lines[i].text;
            execute!(stdout, Print(line), Print("\r\n"))?;
        }

        // Fill remaining lines
        let printed = end.saturating_sub(self.scroll);
        for _ in printed..content_rows {
            execute!(stdout, Print("~\r\n"))?;
        }

        // Footer
        let position = if self.visual_lines.is_empty() {
            "Empty".to_string()
        } else {
            let pct = ((self.scroll + content_rows).min(self.visual_lines.len()) as f64
                / self.visual_lines.len() as f64
                * 100.0) as u32;
            format!("{}%", pct.min(100))
        };
        let footer = format!(
            " q:quit  j/k:\u{2191}\u{2193}  space:pgdn  n/p:next/prev chapter  t:toc | {}",
            position
        );
        let footer_padded = format!("{:<width$}", footer, width = cols as usize);
        execute!(
            stdout,
            cursor::MoveTo(0, rows - 1),
            SetForegroundColor(Color::Black),
            crossterm::style::SetBackgroundColor(Color::DarkGrey),
            Print(&footer_padded),
            ResetColor
        )?;

        stdout.flush()?;
        Ok(())
    }
}

pub fn select_chapter(chapters: &[(String, usize)], current: usize) -> anyhow::Result<Option<usize>> {
    terminal::enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

    let mut selected = current;
    let mut scroll = 0;

    let result = loop {
        let (cols, rows) = terminal::size()?;
        let content_rows = (rows as usize).saturating_sub(3);

        execute!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )?;

        // Header
        let header = " Table of Contents (Enter to select, q to cancel)";
        let header_padded = format!("{:<width$}", header, width = cols as usize);
        execute!(
            stdout,
            SetForegroundColor(Color::Black),
            crossterm::style::SetBackgroundColor(Color::Cyan),
            Print(&header_padded),
            ResetColor,
            Print("\r\n")
        )?;

        // Ensure selected is visible
        if selected < scroll {
            scroll = selected;
        }
        if selected >= scroll + content_rows {
            scroll = selected - content_rows + 1;
        }

        let end = (scroll + content_rows).min(chapters.len());
        for i in scroll..end {
            let (title, _) = &chapters[i];
            if i == selected {
                execute!(
                    stdout,
                    SetAttribute(Attribute::Bold),
                    SetForegroundColor(Color::Cyan),
                    Print(format!("  > {}\r\n", title)),
                    ResetColor,
                    SetAttribute(Attribute::Reset)
                )?;
            } else {
                execute!(stdout, Print(format!("    {}\r\n", title)))?;
            }
        }

        stdout.flush()?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break None,
                KeyCode::Enter => break Some(selected),
                KeyCode::Down | KeyCode::Char('j') => {
                    if selected + 1 < chapters.len() {
                        selected += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    selected = selected.saturating_sub(1);
                }
                _ => {}
            }
        }
    };

    execute!(stdout, cursor::Show, terminal::LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    Ok(result)
}
