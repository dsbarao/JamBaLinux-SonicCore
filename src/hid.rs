use serde::Serialize;

#[derive(Clone, Copy, Debug, Default)]
struct GlobalState {
    usage_page: u32,
    report_size: u32,
    report_count: u32,
    report_id: Option<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ReportKind {
    Input,
    Output,
    Feature,
}

impl ReportKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
            Self::Feature => "feature",
        }
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub id: Option<u8>,
    pub kind: ReportKind,
    pub payload_bits: u32,
    pub payload_bytes: u32,
    pub wire_bytes: u32,
}

#[derive(Debug, Serialize)]
pub struct DescriptorSummary {
    pub descriptor_bytes: usize,
    pub usage_pages: Vec<String>,
    pub collections: u32,
    pub reports: Vec<Report>,
}

#[derive(Debug, Serialize)]
pub struct KnownInputMapping {
    pub report_id: &'static str,
    pub payload: &'static str,
    pub meaning: &'static str,
}

pub fn confirmed_mappings() -> Vec<KnownInputMapping> {
    vec![
        KnownInputMapping {
            report_id: "0x02",
            payload: "0x00",
            meaning: "noise control off",
        },
        KnownInputMapping {
            report_id: "0x02",
            payload: "0x01",
            meaning: "ANC",
        },
        KnownInputMapping {
            report_id: "0x02",
            payload: "0x02",
            meaning: "TalkThru",
        },
        KnownInputMapping {
            report_id: "0x03",
            payload: "0x00",
            meaning: "Bluetooth disconnected",
        },
        KnownInputMapping {
            report_id: "0x03",
            payload: "0x01",
            meaning: "Bluetooth connected",
        },
        KnownInputMapping {
            report_id: "0x03",
            payload: "0x02",
            meaning: "Bluetooth pairing",
        },
        KnownInputMapping {
            report_id: "0x06",
            payload: "0x00",
            meaning: "microphone muted",
        },
        KnownInputMapping {
            report_id: "0x06",
            payload: "0x01",
            meaning: "microphone active",
        },
        KnownInputMapping {
            report_id: "0x07",
            payload: "0x00/0x01",
            meaning: "lighting disabled/enabled",
        },
        KnownInputMapping {
            report_id: "0x08",
            payload: "0x00..0x64",
            meaning: "battery percentage",
        },
        KnownInputMapping {
            report_id: "0x09",
            payload: "0x00",
            meaning: "headset off/disconnected",
        },
        KnownInputMapping {
            report_id: "0x09",
            payload: "0x01",
            meaning: "headset on/connected",
        },
        KnownInputMapping {
            report_id: "0x10",
            payload: "0x00..0x06,0x08,0x0a..0x10",
            meaning: "Game/Chat balance -7..+7",
        },
        KnownInputMapping {
            report_id: "0x2f",
            payload: "bit 1",
            meaning: "HID Telephony Phone Mute pulse",
        },
    ]
}

fn unsigned_value(bytes: &[u8]) -> u32 {
    bytes.iter().enumerate().fold(0, |value, (index, byte)| {
        value | ((*byte as u32) << (index * 8))
    })
}

fn add_report(reports: &mut Vec<Report>, state: GlobalState, kind: ReportKind) {
    let bits = state.report_size.saturating_mul(state.report_count);
    if let Some(report) = reports
        .iter_mut()
        .find(|report| report.id == state.report_id && report.kind == kind)
    {
        report.payload_bits = report.payload_bits.saturating_add(bits);
        report.payload_bytes = report.payload_bits.div_ceil(8);
        report.wire_bytes = report.payload_bytes + u32::from(report.id.is_some());
        return;
    }
    let payload_bytes = bits.div_ceil(8);
    reports.push(Report {
        id: state.report_id,
        kind,
        payload_bits: bits,
        payload_bytes,
        wire_bytes: payload_bytes + u32::from(state.report_id.is_some()),
    });
}

pub fn parse(data: &[u8]) -> Result<DescriptorSummary, String> {
    let mut offset = 0;
    let mut state = GlobalState::default();
    let mut stack = Vec::new();
    let mut reports = Vec::new();
    let mut usage_pages: Vec<u32> = Vec::new();
    let mut collections = 0;

    while offset < data.len() {
        let prefix = data[offset];
        offset += 1;
        if prefix == 0xfe {
            if offset + 2 > data.len() {
                return Err("truncated HID long item header".into());
            }
            let size = data[offset] as usize;
            offset += 2;
            if offset + size > data.len() {
                return Err("truncated HID long item data".into());
            }
            offset += size;
            continue;
        }
        let size = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 4,
            _ => unreachable!(),
        };
        if offset + size > data.len() {
            return Err(format!("truncated HID item at offset {}", offset - 1));
        }
        let value = unsigned_value(&data[offset..offset + size]);
        offset += size;
        let item_type = (prefix >> 2) & 0x03;
        let tag = (prefix >> 4) & 0x0f;

        match (item_type, tag) {
            (1, 0) => {
                state.usage_page = value;
                if !usage_pages.contains(&value) {
                    usage_pages.push(value);
                }
            }
            (1, 7) => state.report_size = value,
            (1, 8) => {
                if value == 0 || value > u8::MAX as u32 {
                    return Err(format!("invalid report ID {value}"));
                }
                state.report_id = Some(value as u8);
            }
            (1, 9) => state.report_count = value,
            (1, 10) => stack.push(state),
            (1, 11) => state = stack.pop().ok_or("HID global pop without push")?,
            (0, 8) => add_report(&mut reports, state, ReportKind::Input),
            (0, 9) => add_report(&mut reports, state, ReportKind::Output),
            (0, 10) => collections += 1,
            (0, 11) => add_report(&mut reports, state, ReportKind::Feature),
            _ => {}
        }
    }
    reports.sort_by_key(|report| {
        (
            report.id.unwrap_or(0),
            match report.kind {
                ReportKind::Input => 0,
                ReportKind::Output => 1,
                ReportKind::Feature => 2,
            },
        )
    });
    usage_pages.sort_unstable();
    Ok(DescriptorSummary {
        descriptor_bytes: data.len(),
        usage_pages: usage_pages
            .iter()
            .map(|page| format!("0x{page:04x}"))
            .collect(),
        collections,
        reports,
    })
}

pub fn interpret_input(bytes: &[u8]) -> Option<String> {
    match bytes {
        [0x02, 0x00] => Some("noise-control=off".into()),
        [0x02, 0x01] => Some("noise-control=anc".into()),
        [0x02, 0x02] => Some("noise-control=talkthru".into()),
        [0x03, 0x00] => Some("bluetooth=disconnected".into()),
        [0x03, 0x01] => Some("bluetooth=connected".into()),
        [0x03, 0x02] => Some("bluetooth=pairing".into()),
        [0x06, 0x00] => Some("microphone=muted".into()),
        [0x06, 0x01] => Some("microphone=active".into()),
        [0x07, 0x00] => Some("lighting=disabled".into()),
        [0x07, 0x01] => Some("lighting=enabled".into()),
        [0x08, percentage @ 0x00..=0x64] => Some(format!("battery={percentage}%")),
        [0x09, 0x00] => Some("headset=off-or-disconnected".into()),
        [0x09, 0x01] => Some("headset=on-or-connected".into()),
        [0x10, value @ 0x00..=0x10] => Some(match value {
            0x00 => "game-chat=-7(chat-max)".into(),
            0x08 => "game-chat=center".into(),
            0x10 => "game-chat=+7(game-max)".into(),
            0x01..=0x06 => format!("game-chat={} (chat)", i16::from(*value) - 7),
            0x0a..=0x0f => format!("game-chat=+{} (game)", i16::from(*value) - 9),
            _ => format!("game-chat=reserved-0x{value:02x}"),
        }),
        [0x2f, value] if value & 0x02 != 0 => Some("phone-mute-usage=asserted".into()),
        [0x2f, _] => Some("phone-mute-usage=released".into()),
        _ => None,
    }
}

pub fn battery_from_feature(bytes: &[u8]) -> Result<u8, String> {
    match bytes {
        [0x49, percentage @ 0x00..=0x64] => Ok(*percentage),
        [0x49, percentage] => Err(format!(
            "battery Feature Report returned out-of-range value {percentage}"
        )),
        [report_id, ..] => Err(format!(
            "expected battery Feature Report 0x49, got 0x{report_id:02x}"
        )),
        [] => Err("battery Feature Report was empty".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        let text = include_str!("../tests/fixtures/quantum810-report-descriptor.hex").trim();
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn parses_quantum_810_fixture() {
        let summary = parse(&fixture()).unwrap();
        assert_eq!(summary.descriptor_bytes, 553);
        assert_eq!(
            summary.usage_pages,
            ["0x0008", "0x0009", "0x000b", "0xff13", "0xff99"]
        );
        assert!(summary.reports.iter().any(|report| {
            report.id == Some(0x06)
                && report.kind == ReportKind::Output
                && report.payload_bytes == 61
                && report.wire_bytes == 62
        }));
        assert!(summary.reports.iter().any(|report| {
            report.id == Some(0x5b)
                && report.kind == ReportKind::Feature
                && report.payload_bytes == 63
        }));
    }

    #[test]
    fn rejects_truncated_item() {
        assert!(parse(&[0x06, 0x13]).is_err());
    }

    #[test]
    fn interprets_confirmed_input_reports() {
        assert_eq!(
            interpret_input(&[0x02, 0x02]),
            Some("noise-control=talkthru".into())
        );
        assert_eq!(
            interpret_input(&[0x03, 0x01]),
            Some("bluetooth=connected".into())
        );
        assert_eq!(
            interpret_input(&[0x06, 0x00]),
            Some("microphone=muted".into())
        );
        assert_eq!(
            interpret_input(&[0x07, 0x01]),
            Some("lighting=enabled".into())
        );
        assert_eq!(interpret_input(&[0x08, 0x55]), Some("battery=85%".into()));
        assert_eq!(
            interpret_input(&[0x10, 0x08]),
            Some("game-chat=center".into())
        );
        assert_eq!(
            interpret_input(&[0x10, 0x01]),
            Some("game-chat=-6 (chat)".into())
        );
        assert_eq!(
            interpret_input(&[0x10, 0x0f]),
            Some("game-chat=+6 (game)".into())
        );
        assert_eq!(
            interpret_input(&[0x2f, 0x02]),
            Some("phone-mute-usage=asserted".into())
        );
        assert_eq!(interpret_input(&[0x08, 0x5a]), Some("battery=90%".into()));
    }

    #[test]
    fn validates_battery_feature_report() {
        assert_eq!(battery_from_feature(&[0x49, 0x55]), Ok(85));
        assert!(battery_from_feature(&[0x49, 0x65]).is_err());
        assert!(battery_from_feature(&[0x48, 0x55]).is_err());
    }

    #[test]
    fn rejects_empty_battery_feature_report() {
        assert_eq!(
            battery_from_feature(&[]),
            Err("battery Feature Report was empty".into())
        );
    }
}
