//! Scanning the sheet directly, instead of asking for a file.
//!
//! Driving the scanner is worth more than the step it saves. Onionskin has to
//! find the paper's outline to know how big it is and how far it is turned, and
//! the settings that make that possible are exactly the ones a scanning
//! program turns on by default and gets wrong for us:
//!
//! * **auto-crop** throws away the scanner backing around the sheet, and with
//!   it the outline this whole workflow is measured from;
//! * **auto-deskew** straightens the image, which sounds helpful and is not —
//!   it silently rewrites the geometry the delta has to match;
//! * **auto-rotate** can turn the page a quarter turn without saying so.
//!
//! Asking for the scan ourselves means asking for a plain one.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum AcquireError {
    #[error("{0}")]
    NoScannerTool(String),
    #[error("could not run {tool}: {source}")]
    Launch {
        tool: String,
        source: std::io::Error,
    },
    #[error("{tool} failed: {message}")]
    Failed { tool: String, message: String },
    #[error("no scanner was found. Check it is switched on and plugged in.")]
    NoDevice,
}

/// A scanner the machine can see.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub name: String,
    pub description: String,
}

/// What to ask the scanner for.
#[derive(Debug, Clone)]
pub struct AcquireOptions {
    /// Which scanner, when there is more than one. `None` takes the first.
    pub device: Option<String>,
    /// Dots per inch. 300 is plenty: the sheet's outline is a centimetre-scale
    /// feature, and a finer scan costs time and memory for no better fix.
    pub resolution: u32,
    /// Scan in colour rather than greyscale.
    pub colour: bool,
}

impl Default for AcquireOptions {
    fn default() -> Self {
        Self {
            device: None,
            resolution: 300,
            colour: false,
        }
    }
}

/// The SANE command-line scanner, which is how Linux and macOS reach a scanner.
const TOOL: &str = "scanimage";

/// Can this machine reach a scanner at all?
///
/// Worth asking before anything else is printed: being told how to lay the
/// sheet on the glass, and then that there is no scanner, wastes the reader's
/// attention in the wrong order.
pub fn scanning_available() -> bool {
    tool_available()
}

/// Why scanning is unavailable, for the message.
pub fn unavailable_reason() -> String {
    missing_tool_message()
}

fn tool_available() -> bool {
    Command::new(TOOL)
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// Guidance for a machine with no scanning tool, in its own terms.
fn missing_tool_message() -> String {
    let install = if cfg!(target_os = "macos") {
        "Install it with:  brew install sane-backends"
    } else if cfg!(target_os = "windows") {
        "Windows scanners are not reachable this way yet — scan with the software \
         that came with the scanner, save a PNG or JPEG, and pass that file instead."
    } else {
        "Install it with your package manager, for example:  \
         sudo apt install sane-utils"
    };
    format!(
        "no scanning tool found. Onionskin drives a scanner through SANE's \
         '{TOOL}'.\n    {install}\n    You can also scan with any program you \
         like and pass the image file instead."
    )
}

/// List the scanners this machine can see.
pub fn list_devices() -> Result<Vec<Device>, AcquireError> {
    if !tool_available() {
        return Err(AcquireError::NoScannerTool(missing_tool_message()));
    }
    let output = Command::new(TOOL)
        .args(["--formatted-device-list", "%d|%v %m%n"])
        .output()
        .map_err(|source| AcquireError::Launch {
            tool: TOOL.into(),
            source,
        })?;

    if !output.status.success() {
        return Err(AcquireError::Failed {
            tool: TOOL.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }
    Ok(parse_device_list(&String::from_utf8_lossy(&output.stdout)))
}

/// Parse the device list. Kept separate so it can be tested without a scanner.
pub fn parse_device_list(text: &str) -> Vec<Device> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, description) = line.split_once('|')?;
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            Some(Device {
                name: name.to_string(),
                description: description.trim().to_string(),
            })
        })
        .collect()
}

/// The arguments Onionskin asks a scanner for.
///
/// Separated from running it so the choices can be tested and read.
pub fn scan_arguments(options: &AcquireOptions, out: &Path) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Some(device) = &options.device {
        args.push("--device-name".into());
        args.push(device.clone());
    }
    args.push("--format=png".into());
    args.push(format!("--resolution={}", options.resolution));
    args.push(format!(
        "--mode={}",
        if options.colour { "Color" } else { "Gray" }
    ));
    args.push("--output-file".into());
    args.push(out.to_string_lossy().to_string());
    args
}

/// Scan a sheet to `out`.
pub fn acquire(options: &AcquireOptions, out: &Path) -> Result<PathBuf, AcquireError> {
    if !tool_available() {
        return Err(AcquireError::NoScannerTool(missing_tool_message()));
    }

    let output = Command::new(TOOL)
        .args(scan_arguments(options, out))
        .output()
        .map_err(|source| AcquireError::Launch {
            tool: TOOL.into(),
            source,
        })?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if message.contains("no SANE devices") || message.contains("Invalid argument") {
            return Err(AcquireError::NoDevice);
        }
        return Err(AcquireError::Failed {
            tool: TOOL.into(),
            message,
        });
    }
    if !out.is_file() {
        return Err(AcquireError::Failed {
            tool: TOOL.into(),
            message: "the scan produced no image".into(),
        });
    }
    Ok(out.to_path_buf())
}

/// What to tell someone before they lift the lid.
pub const PLACEMENT_ADVICE: &str = "\
Before scanning
  * Lay the sheet squarely on the glass, but do not fret over a degree or
    two — Onionskin measures the tilt and corrects for it.
  * Leave the lid closed, and leave a margin of the scanner's own backing
    visible around the paper: that outline is what the page is measured from.
  * Turn OFF auto-crop, auto-deskew and auto-rotate if your scanner offers
    them. They rewrite the geometry the delta has to match.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_list_is_parsed() {
        let text = "plustek:libusb:001:005|Canon CanoScan LiDE 220\n\
                    epson2:net:192.168.1.9|Epson Perfection V600\n";
        let devices = parse_device_list(text);

        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].name, "plustek:libusb:001:005");
        assert_eq!(devices[0].description, "Canon CanoScan LiDE 220");
        assert_eq!(devices[1].name, "epson2:net:192.168.1.9");
    }

    #[test]
    fn a_ragged_device_list_does_not_confuse_it() {
        let text = "\n   \nbroken line with no separator\n|no name\ngood:dev|A Scanner\n";
        let devices = parse_device_list(text);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "good:dev");
    }

    #[test]
    fn an_empty_list_is_no_devices() {
        assert!(parse_device_list("").is_empty());
        assert!(parse_device_list("\n\n").is_empty());
    }

    #[test]
    fn the_scan_asks_for_a_plain_image() {
        let options = AcquireOptions::default();
        let args = scan_arguments(&options, Path::new("/tmp/out.png"));

        assert!(args.contains(&"--format=png".to_string()));
        assert!(args.contains(&"--resolution=300".to_string()));
        assert!(args.contains(&"--mode=Gray".to_string()));
        assert!(args.contains(&"/tmp/out.png".to_string()));
        // Nothing that would crop, straighten or turn the page: those would
        // throw away the outline the whole workflow is measured from.
        assert!(!args.iter().any(|a| a.contains("crop")));
        assert!(!args.iter().any(|a| a.contains("deskew")));
        assert!(!args.iter().any(|a| a.contains("rotate")));
    }

    #[test]
    fn a_named_device_is_passed_through() {
        let options = AcquireOptions {
            device: Some("epson2:net:192.168.1.9".into()),
            resolution: 600,
            colour: true,
        };
        let args = scan_arguments(&options, Path::new("/tmp/o.png"));

        assert!(args.contains(&"--device-name".to_string()));
        assert!(args.contains(&"epson2:net:192.168.1.9".to_string()));
        assert!(args.contains(&"--resolution=600".to_string()));
        assert!(args.contains(&"--mode=Color".to_string()));
    }

    #[test]
    fn the_advice_covers_what_actually_goes_wrong() {
        // Auto-crop and auto-deskew are the settings that quietly ruin this.
        assert!(PLACEMENT_ADVICE.contains("auto-crop"));
        assert!(PLACEMENT_ADVICE.contains("auto-deskew"));
        assert!(PLACEMENT_ADVICE.contains("margin"));
    }

    #[test]
    fn a_machine_without_the_tool_is_told_what_to_install() {
        let message = missing_tool_message();
        assert!(message.contains("scanimage"));
        // And that they are not stuck: a file works just as well.
        assert!(message.contains("pass the image file instead"));
    }
}
