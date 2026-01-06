//! Output formatting and error handling utilities for rpm
//!
//! This module provides consistent styling for terminal output including:
//! - Color constants for ANSI terminal colors
//! - Helper functions for success, warning, error, and info messages
//! - Structured error types with helpful suggestions

use std::fmt;
use std::io::{self, IsTerminal};

// ============================================================================
// ANSI Color Constants
// ============================================================================

pub mod colors {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";

    // Standard colors
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const GRAY: &str = "\x1b[90m";

    // Bold variants
    pub const BOLD_RED: &str = "\x1b[1;31m";
    pub const BOLD_GREEN: &str = "\x1b[1;32m";
    pub const BOLD_YELLOW: &str = "\x1b[1;33m";
    pub const BOLD_CYAN: &str = "\x1b[1;36m";
    pub const BOLD_MAGENTA: &str = "\x1b[1;35m";
}

// ============================================================================
// Symbols for consistent UI
// ============================================================================

pub mod symbols {
    pub const SUCCESS: &str = "✓";
    pub const ERROR: &str = "✗";
    pub const WARNING: &str = "!";
    pub const INFO: &str = "•";
    pub const ARROW_UP: &str = "↑";
    pub const ARROW_RIGHT: &str = "→";
    pub const PLUS: &str = "+";
    pub const MINUS: &str = "-";
    pub const TREE_BRANCH: &str = "├─";
    pub const TREE_END: &str = "└─";
    pub const SEPARATOR: &str = "│";
}

// ============================================================================
// Output Helper Functions
// ============================================================================

/// Check if colors should be used based on terminal and environment
pub fn should_use_colors() -> bool {
    // Respect NO_COLOR environment variable (https://no-color.org/)
    if std::env::var("NO_COLOR").is_ok() {
        return false;
    }

    // Respect FORCE_COLOR environment variable
    if std::env::var("FORCE_COLOR").is_ok() {
        return true;
    }

    // Check if stdout is a terminal
    io::stdout().is_terminal()
}

/// Strip ANSI color codes from a string if colors are disabled
pub fn maybe_strip_colors(s: &str) -> String {
    if should_use_colors() {
        s.to_string()
    } else {
        // Simple regex-free ANSI stripping
        let mut result = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                // Skip until 'm'
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                result.push(c);
            }
        }
        result
    }
}

/// Print a success message with green checkmark
pub fn success(msg: &str) {
    use colors::*;
    use symbols::*;
    let output = format!("{GREEN}{SUCCESS}{RESET} {msg}");
    println!("{}", maybe_strip_colors(&output));
}

/// Print a warning message with yellow exclamation
pub fn warning(msg: &str) {
    use colors::*;
    let output = format!("{YELLOW}warn:{RESET} {msg}");
    println!("{}", maybe_strip_colors(&output));
}

/// Print an error message with red X
pub fn error(msg: &str) {
    use colors::*;
    let output = format!("{BOLD_RED}error:{RESET} {msg}");
    eprintln!("{}", maybe_strip_colors(&output));
}

/// Print an info message
pub fn info(msg: &str) {
    use colors::*;
    let output = format!("{CYAN}info:{RESET} {msg}");
    println!("{}", maybe_strip_colors(&output));
}

/// Print a hint/suggestion message (indented, dimmed)
pub fn hint(msg: &str) {
    use colors::*;
    let output = format!("      {GRAY}{msg}{RESET}");
    println!("{}", maybe_strip_colors(&output));
}

/// Print a package added message
pub fn package_added(name: &str, version: &str) {
    use colors::*;
    use symbols::*;
    let output = format!("{GREEN}{PLUS}{RESET} {BOLD}{name}{RESET}@{GRAY}{version}{RESET}");
    println!("{}", maybe_strip_colors(&output));
}

/// Print a package removed message
pub fn package_removed(name: &str) {
    use colors::*;
    use symbols::*;
    let output = format!("{RED}{MINUS}{RESET} {BOLD}{name}{RESET}");
    println!("{}", maybe_strip_colors(&output));
}

/// Print a package updated message
pub fn package_updated(name: &str, old_version: &str, new_version: &str) {
    use colors::*;
    use symbols::*;
    let output = format!(
        "{CYAN}{ARROW_UP}{RESET} {BOLD}{name}{RESET} {GRAY}{old_version}{RESET} {ARROW_RIGHT} {GREEN}{new_version}{RESET}"
    );
    println!("{}", maybe_strip_colors(&output));
}

/// Format an install summary line
pub fn format_summary(installed: usize, cached: usize, linked: Option<usize>) -> String {
    use colors::*;
    use symbols::*;

    let mut parts = Vec::new();
    if installed > 0 {
        parts.push(format!("{GREEN}+{installed}{RESET} installed"));
    }
    if cached > 0 {
        parts.push(format!("{YELLOW}{cached}{RESET} cached"));
    }
    if let Some(l) = linked {
        if l > 0 {
            parts.push(format!("{CYAN}{l}{RESET} linked"));
        }
    }

    if parts.is_empty() {
        format!("{GRAY}No packages to install{RESET}")
    } else {
        parts.join(&format!("  {GRAY}{SEPARATOR}{RESET}  "))
    }
}

// ============================================================================
// Structured Error Types with Suggestions
// ============================================================================

/// Errors that can occur during package operations with helpful suggestions
#[derive(Debug)]
pub enum RpmError {
    /// Package not found in registry
    PackageNotFound {
        name: String,
        suggestions: Vec<String>,
    },

    /// No version matches the requested range
    VersionNotFound {
        name: String,
        requested: String,
        available: Vec<String>,
    },

    /// Network error while fetching package
    NetworkError {
        name: String,
        status: Option<u16>,
        message: String,
    },

    /// Failed to parse package metadata
    ParseError { name: String, message: String },

    /// Script not found in package.json
    ScriptNotFound {
        script: String,
        available: Vec<String>,
    },

    /// Binary not found in package
    BinaryNotFound { package: String, binary: String },

    /// Workspace error
    WorkspaceError { message: String },

    /// Generic error with optional hint
    Other {
        message: String,
        hint: Option<String>,
    },
}

impl fmt::Display for RpmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use colors::*;

        match self {
            RpmError::PackageNotFound { name, suggestions } => {
                write!(f, "Package {BOLD}'{name}'{RESET} not found in registry")?;
                if !suggestions.is_empty() {
                    write!(f, "\n\n      {GRAY}Did you mean one of these?{RESET}")?;
                    for suggestion in suggestions.iter().take(3) {
                        write!(f, "\n        {CYAN}•{RESET} {suggestion}")?;
                    }
                }
                write!(
                    f,
                    "\n\n      {GRAY}Tip: Check the package name spelling or search at https://www.npmjs.com{RESET}"
                )?;
                Ok(())
            }

            RpmError::VersionNotFound {
                name,
                requested,
                available,
            } => {
                write!(
                    f,
                    "No version of {BOLD}'{name}'{RESET} matches {YELLOW}'{requested}'{RESET}"
                )?;
                if !available.is_empty() {
                    write!(f, "\n\n      {GRAY}Available versions:{RESET}")?;
                    for version in available.iter().take(5) {
                        write!(f, "\n        {CYAN}•{RESET} {version}")?;
                    }
                    if available.len() > 5 {
                        write!(
                            f,
                            "\n        {GRAY}... and {} more{RESET}",
                            available.len() - 5
                        )?;
                    }
                }
                write!(
                    f,
                    "\n\n      {GRAY}Tip: Use 'rpm info {name}' to see all available versions{RESET}"
                )?;
                Ok(())
            }

            RpmError::NetworkError {
                name,
                status,
                message,
            } => {
                write!(f, "Failed to fetch package {BOLD}'{name}'{RESET}")?;
                if let Some(code) = status {
                    write!(f, " (HTTP {code})")?;
                }
                write!(f, ": {message}")?;
                write!(
                    f,
                    "\n\n      {GRAY}Tip: Check your internet connection or try again later{RESET}"
                )?;
                if status == &Some(404) {
                    write!(
                        f,
                        "\n      {GRAY}     The package may have been unpublished or the name is incorrect{RESET}"
                    )?;
                }
                Ok(())
            }

            RpmError::ParseError { name, message } => {
                write!(
                    f,
                    "Failed to parse metadata for {BOLD}'{name}'{RESET}: {message}"
                )?;
                write!(
                    f,
                    "\n\n      {GRAY}Tip: This may be a temporary registry issue. Try again later{RESET}"
                )?;
                write!(
                    f,
                    "\n      {GRAY}     or report this issue at https://github.com/lassejlv/rpm{RESET}"
                )?;
                Ok(())
            }

            RpmError::ScriptNotFound { script, available } => {
                write!(f, "Script {BOLD}'{script}'{RESET} not found")?;
                if available.is_empty() {
                    write!(
                        f,
                        "\n\n      {GRAY}No scripts defined in package.json{RESET}"
                    )?;
                } else {
                    write!(f, "\n\n      {GRAY}Available scripts:{RESET}")?;
                    for s in available.iter().take(10) {
                        write!(f, "\n        {CYAN}•{RESET} {s}")?;
                    }
                    if available.len() > 10 {
                        write!(
                            f,
                            "\n        {GRAY}... and {} more{RESET}",
                            available.len() - 10
                        )?;
                    }
                }
                write!(
                    f,
                    "\n\n      {GRAY}Tip: Run 'rpm run' to see all available scripts{RESET}"
                )?;
                Ok(())
            }

            RpmError::BinaryNotFound { package, binary } => {
                write!(
                    f,
                    "Binary {BOLD}'{binary}'{RESET} not found in package {BOLD}'{package}'{RESET}"
                )?;
                write!(
                    f,
                    "\n\n      {GRAY}Tip: The package may not provide an executable binary{RESET}"
                )?;
                write!(f, "\n      {GRAY}     Check the package documentation at https://www.npmjs.com/package/{package}{RESET}")?;
                Ok(())
            }

            RpmError::WorkspaceError { message } => {
                write!(f, "{message}")?;
                write!(
                    f,
                    "\n\n      {GRAY}Tip: Make sure you're in a workspace root with 'workspaces' field in package.json{RESET}"
                )?;
                Ok(())
            }

            RpmError::Other { message, hint } => {
                write!(f, "{message}")?;
                if let Some(h) = hint {
                    write!(f, "\n\n      {GRAY}Tip: {h}{RESET}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for RpmError {}

// ============================================================================
// Progress Reporting Helpers
// ============================================================================

/// Format a duration in a human-readable way
pub fn format_duration(secs: f64) -> String {
    if secs < 1.0 {
        format!("{:.0}ms", secs * 1000.0)
    } else if secs < 60.0 {
        format!("{:.2}s", secs)
    } else {
        let mins = (secs / 60.0).floor() as u64;
        let remaining_secs = secs % 60.0;
        format!("{}m {:.1}s", mins, remaining_secs)
    }
}

/// Format bytes in a human-readable way
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Create a progress status line for package installation
pub fn format_progress_status(resolving: usize, installing: usize, cached: usize) -> String {
    use colors::*;
    use symbols::*;

    let mut parts = vec![format!(
        "{BOLD}Resolving{RESET} {CYAN}{resolving}{RESET} packages"
    )];

    parts.push(format!(
        "{BOLD}Installing{RESET} {GREEN}{installing}{RESET}"
    ));

    if cached > 0 {
        parts.push(format!("{GRAY}Cached{RESET} {YELLOW}{cached}{RESET}"));
    }

    parts.join(&format!("  {GRAY}{SEPARATOR}{RESET}  "))
}

/// Spinner tick characters for consistent animation
pub const SPINNER_CHARS: &str = "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏";

/// Progress bar characters
pub const PROGRESS_CHARS: &str = "━╸─";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0.5), "500ms");
        assert_eq!(format_duration(1.5), "1.50s");
        assert_eq!(format_duration(65.0), "1m 5.0s");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1500), "1.5 KB");
        assert_eq!(format_bytes(1500000), "1.43 MB");
    }
}
