//! Pure renderer for the experimental spatial (binaural) filter-chain config.
//!
//! Phase 2 boundary: this module ONLY renders deterministic PipeWire
//! filter-chain configuration text from a validated HRIR path. It never spawns
//! `pipewire`, connects to a running graph, edits global PipeWire files, changes
//! routing, or touches the default sink — that lifecycle belongs to the
//! persistent service in a later phase. Until that phase wires it in, the
//! binary itself does not call these items, hence the crate-level dead-code
//! allowance.
//!
//! The graph faithfully reproduces the layout of the official installed example
//! `/usr/share/pipewire/filter-chain/sink-virtual-surround-7.1-hesuvi.conf`:
//! eight surround inputs feed sixteen HRIR convolvers (fourteen distinct WAV
//! channels; the two LFE convolvers reuse the FC responses), mixed down to a
//! stereo pair that is handed to the existing equalizer sink.
//!
//! Rendering is fail-closed with respect to the one caller-controlled value in
//! the config — the HRIR path. A path that cannot be expressed exactly is a
//! hard error rather than an approximation, and path text is never re-scanned
//! for template markers.
#![allow(dead_code)]

use std::path::Path;

/// Stable, vendor-neutral node name of the spatial capture sink (the 7.1 input
/// applications select).
pub const SPATIAL_SINK_NODE: &str = "jambalinux-soniccore-game-spatial";

/// Stable, vendor-neutral node name of the spatial stereo output.
pub const SPATIAL_OUTPUT_NODE: &str = "jambalinux-soniccore-game-spatial-output";

/// The existing equalizer sink the spatial output must feed. It has to match
/// `crate::pipewire`'s virtual sink name for the spatial -> EQ -> headset chain
/// to form; it is duplicated (not imported) so this renderer stays decoupled
/// from the equalizer backend.
pub const TARGET_EQUALIZER_SINK: &str = "jambalinux-soniccore-game-equalizer";

/// The eight surround inputs, in the exact channel order the sink advertises.
pub const INPUT_POSITIONS: [&str; 8] = ["FL", "FR", "FC", "LFE", "RL", "RR", "SL", "SR"];

/// The eight input duplicators, one per surround channel.
const COPIES: [&str; 8] = [
    "copyFL", "copyFR", "copyFC", "copyLFE", "copyRL", "copyRR", "copySL", "copySR",
];

/// One HRIR convolver: its node name and which WAV channel it reads.
struct Convolver {
    name: &'static str,
    channel: u8,
}

/// The sixteen convolvers of the official 7.1 HeSuVi layout. Fourteen read
/// distinct HRIR channels (0..=13); the final two treat LFE as FC by reusing
/// channels 6 and 13.
const CONVOLVERS: [Convolver; 16] = [
    Convolver {
        name: "convFL_L",
        channel: 0,
    },
    Convolver {
        name: "convFL_R",
        channel: 1,
    },
    Convolver {
        name: "convSL_L",
        channel: 2,
    },
    Convolver {
        name: "convSL_R",
        channel: 3,
    },
    Convolver {
        name: "convRL_L",
        channel: 4,
    },
    Convolver {
        name: "convRL_R",
        channel: 5,
    },
    Convolver {
        name: "convFC_L",
        channel: 6,
    },
    Convolver {
        name: "convFR_R",
        channel: 7,
    },
    Convolver {
        name: "convFR_L",
        channel: 8,
    },
    Convolver {
        name: "convSR_R",
        channel: 9,
    },
    Convolver {
        name: "convSR_L",
        channel: 10,
    },
    Convolver {
        name: "convRR_R",
        channel: 11,
    },
    Convolver {
        name: "convRR_L",
        channel: 12,
    },
    Convolver {
        name: "convFC_R",
        channel: 13,
    },
    Convolver {
        name: "convLFE_L",
        channel: 6,
    },
    Convolver {
        name: "convLFE_R",
        channel: 13,
    },
];

/// Links from each input duplicator into its convolvers (`copy:Out -> conv:In`).
const INPUT_LINKS: [(&str, &str); 16] = [
    ("copyFL", "convFL_L"),
    ("copyFL", "convFL_R"),
    ("copySL", "convSL_L"),
    ("copySL", "convSL_R"),
    ("copyRL", "convRL_L"),
    ("copyRL", "convRL_R"),
    ("copyFC", "convFC_L"),
    ("copyFR", "convFR_R"),
    ("copyFR", "convFR_L"),
    ("copySR", "convSR_R"),
    ("copySR", "convSR_L"),
    ("copyRR", "convRR_R"),
    ("copyRR", "convRR_L"),
    ("copyFC", "convFC_R"),
    ("copyLFE", "convLFE_L"),
    ("copyLFE", "convLFE_R"),
];

/// Links from each convolver into a stereo mixer input slot
/// (`conv:Out -> mixL/mixR:In N`).
const OUTPUT_LINKS: [(&str, &str, u8); 16] = [
    ("convFL_L", "mixL", 1),
    ("convFL_R", "mixR", 1),
    ("convSL_L", "mixL", 2),
    ("convSL_R", "mixR", 2),
    ("convRL_L", "mixL", 3),
    ("convRL_R", "mixR", 3),
    ("convFC_L", "mixL", 4),
    ("convFC_R", "mixR", 4),
    ("convFR_R", "mixR", 5),
    ("convFR_L", "mixL", 5),
    ("convSR_R", "mixR", 6),
    ("convSR_L", "mixL", 6),
    ("convRR_R", "mixR", 7),
    ("convRR_L", "mixL", 7),
    ("convLFE_R", "mixR", 8),
    ("convLFE_L", "mixL", 8),
];

/// Escapes a value for inclusion inside a double-quoted PipeWire config string.
/// Mirrors `crate::pipewire::pipewire_string` and additionally neutralizes the
/// control characters that would otherwise break the config text.
fn escape_config_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// The `{marker}` placeholders [`TEMPLATE`] defines. Every marker is
/// brace-delimited, so no marker can be a prefix of another and the
/// substitution order is irrelevant.
const TEMPLATE_MARKERS: [&str; 7] = [
    "{nodes}",
    "{links}",
    "{inputs}",
    "{positions}",
    "{sink}",
    "{output}",
    "{target}",
];

/// Substitutes every `{marker}` in `template` in a single left-to-right pass.
///
/// Substituted values are appended verbatim and never re-scanned, so a value
/// that happens to contain a marker — an HRIR path with a literal `{target}` in
/// it, say — cannot be mistaken for a placeholder and rewritten. A `{` that does
/// not begin a known marker (every structural brace in the config) is copied
/// through untouched.
fn render_template(template: &str, values: &[(&str, &str)]) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        rendered.push_str(&rest[..open]);
        let at_brace = &rest[open..];
        match values
            .iter()
            .find(|(marker, _)| at_brace.starts_with(marker))
        {
            Some((marker, value)) => {
                rendered.push_str(value);
                rest = &at_brace[marker.len()..];
            }
            None => {
                rendered.push('{');
                rest = &at_brace[1..];
            }
        }
    }
    rendered.push_str(rest);
    rendered
}

/// Renders the complete, deterministic filter-chain config for the spatial sink
/// feeding the existing equalizer. Pure: it only formats text and performs no
/// I/O, process spawning, or PipeWire interaction.
///
/// Fails when `hrir_path` is not valid UTF-8. A lossy conversion would replace
/// the undecodable bytes with U+FFFD and hand PipeWire a config naming a
/// *different* file, so the error is raised instead of silently rewriting the
/// user's path.
pub fn render_filter_chain_config(hrir_path: &Path) -> Result<String, String> {
    let hrir_text = hrir_path.to_str().ok_or_else(|| {
        format!(
            "the HRIR path {} is not valid UTF-8, so it cannot be written into a PipeWire \
             filter-chain configuration without changing which file it names; rename or move \
             the dataset so every component of its path is valid UTF-8",
            hrir_path.display()
        )
    })?;

    let hrir = escape_config_string(hrir_text);
    let target = escape_config_string(TARGET_EQUALIZER_SINK);

    let mut nodes = String::new();
    for copy in COPIES {
        nodes.push_str(&format!(
            "                    {{ type = builtin label = copy name = {copy} }}\n"
        ));
    }
    for convolver in &CONVOLVERS {
        nodes.push_str(&format!(
            "                    {{ type = builtin label = convolver name = {} config = {{ filename = \"{hrir}\" channel = {} }} }}\n",
            convolver.name, convolver.channel
        ));
    }
    nodes.push_str("                    { type = builtin label = mixer name = mixL }\n");
    nodes.push_str("                    { type = builtin label = mixer name = mixR }\n");

    let mut links = String::new();
    for (output, input) in INPUT_LINKS {
        links.push_str(&format!(
            "                    {{ output = \"{output}:Out\" input = \"{input}:In\" }}\n"
        ));
    }
    for (output, mixer, index) in OUTPUT_LINKS {
        links.push_str(&format!(
            "                    {{ output = \"{output}:Out\" input = \"{mixer}:In {index}\" }}\n"
        ));
    }

    let inputs = COPIES
        .iter()
        .map(|copy| format!("\"{copy}:In\""))
        .collect::<Vec<_>>()
        .join(" ");
    let positions = INPUT_POSITIONS.join(" ");

    Ok(render_template(
        TEMPLATE,
        &[
            ("{nodes}", nodes.as_str()),
            ("{links}", links.as_str()),
            ("{inputs}", inputs.as_str()),
            ("{positions}", positions.as_str()),
            ("{sink}", SPATIAL_SINK_NODE),
            ("{output}", SPATIAL_OUTPUT_NODE),
            ("{target}", target.as_str()),
        ],
    ))
}

/// The config skeleton. Literal braces are structural; only the distinct
/// `{marker}` tokens listed in [`TEMPLATE_MARKERS`] are substituted, each
/// exactly once, by [`render_template`].
const TEMPLATE: &str = r#"# Generated by JamBaLinux SonicCore spatial backend. Do not edit while spatial audio is active.
# Experimental open binaural rendering; no vendor spatial algorithm is used or implied.
context.properties = { log.level = 0 }
context.spa-libs = {
    audio.convert.* = audioconvert/libspa-audioconvert
    support.* = support/libspa-support
}
context.modules = [
    { name = libpipewire-module-rt flags = [ ifexists nofail ] }
    { name = libpipewire-module-protocol-native }
    { name = libpipewire-module-client-node }
    { name = libpipewire-module-adapter }
    { name = libpipewire-module-filter-chain
        args = {
            node.description = "JamBaLinux Game Spatial"
            media.name = "JamBaLinux Game Spatial"
            filter.graph = {
                nodes = [
{nodes}                ]
                links = [
{links}                ]
                inputs = [ {inputs} ]
                outputs = [ "mixL:Out" "mixR:Out" ]
            }
            capture.props = {
                node.name = "{sink}"
                node.description = "JamBaLinux Game Spatial"
                media.class = Audio/Sink
                node.virtual = true
                priority.session = 0
                priority.driver = 0
                audio.channels = 8
                audio.position = [ {positions} ]
            }
            playback.props = {
                node.name = "{output}"
                node.passive = true
                audio.channels = 2
                audio.position = [ FL FR ]
                target.object = "{target}"
            }
        }
    }
]
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TEST_HRIR: &str = "/home/user/.config/jambalinux-soniccore/spatial/hrtf/hrir.wav";

    fn render() -> String {
        render_hrir(TEST_HRIR)
    }

    /// Renders the config for an arbitrary (valid UTF-8) HRIR path.
    fn render_hrir(path: &str) -> String {
        let rendered = render_filter_chain_config(&PathBuf::from(path));
        rendered.expect("a UTF-8 HRIR path renders")
    }

    // -----------------------------------------------------------------------
    // Explicit expectation tables.
    //
    // These duplicate the module's constants on purpose: they are the test's
    // own transcription of the official
    // `sink-virtual-surround-7.1-hesuvi.conf` layout, so any edit to the
    // constants — reorder, rename, or channel change — has to be made here too,
    // deliberately, instead of silently passing a spot check.

    const EXPECTED_COPIES: [&str; 8] = [
        "copyFL", "copyFR", "copyFC", "copyLFE", "copyRL", "copyRR", "copySL", "copySR",
    ];

    const EXPECTED_CONVOLVERS: [(&str, u8); 16] = [
        ("convFL_L", 0),
        ("convFL_R", 1),
        ("convSL_L", 2),
        ("convSL_R", 3),
        ("convRL_L", 4),
        ("convRL_R", 5),
        ("convFC_L", 6),
        ("convFR_R", 7),
        ("convFR_L", 8),
        ("convSR_R", 9),
        ("convSR_L", 10),
        ("convRR_R", 11),
        ("convRR_L", 12),
        ("convFC_R", 13),
        // LFE is treated as FC in this first version: these two reuse the FC
        // responses rather than reading channels of their own.
        ("convLFE_L", 6),
        ("convLFE_R", 13),
    ];

    const EXPECTED_INPUT_LINKS: [(&str, &str); 16] = [
        ("copyFL", "convFL_L"),
        ("copyFL", "convFL_R"),
        ("copySL", "convSL_L"),
        ("copySL", "convSL_R"),
        ("copyRL", "convRL_L"),
        ("copyRL", "convRL_R"),
        ("copyFC", "convFC_L"),
        ("copyFR", "convFR_R"),
        ("copyFR", "convFR_L"),
        ("copySR", "convSR_R"),
        ("copySR", "convSR_L"),
        ("copyRR", "convRR_R"),
        ("copyRR", "convRR_L"),
        ("copyFC", "convFC_R"),
        ("copyLFE", "convLFE_L"),
        ("copyLFE", "convLFE_R"),
    ];

    const EXPECTED_OUTPUT_LINKS: [(&str, &str, u8); 16] = [
        ("convFL_L", "mixL", 1),
        ("convFL_R", "mixR", 1),
        ("convSL_L", "mixL", 2),
        ("convSL_R", "mixR", 2),
        ("convRL_L", "mixL", 3),
        ("convRL_R", "mixR", 3),
        ("convFC_L", "mixL", 4),
        ("convFC_R", "mixR", 4),
        ("convFR_R", "mixR", 5),
        ("convFR_L", "mixL", 5),
        ("convSR_R", "mixR", 6),
        ("convSR_L", "mixL", 6),
        ("convRR_R", "mixR", 7),
        ("convRR_L", "mixL", 7),
        ("convLFE_R", "mixR", 8),
        ("convLFE_L", "mixL", 8),
    ];

    /// The exact `nodes = [ ... ]` body the tables above imply, for `hrir`.
    fn expected_nodes_block(hrir: &str) -> String {
        let mut block = String::new();
        for copy in EXPECTED_COPIES {
            block.push_str(&format!(
                "                    {{ type = builtin label = copy name = {copy} }}\n"
            ));
        }
        for (name, channel) in EXPECTED_CONVOLVERS {
            block.push_str(&format!(
                "                    {{ type = builtin label = convolver name = {name} config = {{ filename = \"{hrir}\" channel = {channel} }} }}\n"
            ));
        }
        block.push_str("                    { type = builtin label = mixer name = mixL }\n");
        block.push_str("                    { type = builtin label = mixer name = mixR }\n");
        block
    }

    /// The exact `links = [ ... ]` body the tables above imply.
    fn expected_links_block() -> String {
        let mut block = String::new();
        for (output, input) in EXPECTED_INPUT_LINKS {
            block.push_str(&format!(
                "                    {{ output = \"{output}:Out\" input = \"{input}:In\" }}\n"
            ));
        }
        for (output, mixer, index) in EXPECTED_OUTPUT_LINKS {
            block.push_str(&format!(
                "                    {{ output = \"{output}:Out\" input = \"{mixer}:In {index}\" }}\n"
            ));
        }
        block
    }

    #[test]
    fn render_is_deterministic() {
        assert_eq!(render(), render());
    }

    #[test]
    fn capture_declares_eight_surround_inputs_in_order() {
        let config = render();
        assert!(config.contains(&format!("node.name = \"{SPATIAL_SINK_NODE}\"")));
        assert!(config.contains("media.class = Audio/Sink"));
        assert!(config.contains("audio.channels = 8"));
        assert!(config.contains("audio.position = [ FL FR FC LFE RL RR SL SR ]"));
        assert!(config.contains(
            "inputs = [ \"copyFL:In\" \"copyFR:In\" \"copyFC:In\" \"copyLFE:In\" \
             \"copyRL:In\" \"copyRR:In\" \"copySL:In\" \"copySR:In\" ]"
        ));
    }

    #[test]
    fn playback_is_stereo_and_targets_only_the_equalizer() {
        let config = render();
        assert!(config.contains(&format!("node.name = \"{SPATIAL_OUTPUT_NODE}\"")));
        assert!(config.contains("node.passive = true"));
        assert!(config.contains("audio.channels = 2"));
        assert!(config.contains("audio.position = [ FL FR ]"));
        assert!(config.contains("outputs = [ \"mixL:Out\" \"mixR:Out\" ]"));
        assert!(config.contains(&format!("target.object = \"{TARGET_EQUALIZER_SINK}\"")));
        // Exactly one routing target, and it is the equalizer.
        assert_eq!(config.matches("target.object").count(), 1);
    }

    #[test]
    fn all_sixteen_convolvers_map_to_the_expected_channels() {
        let config = render();
        assert_eq!(config.matches("label = copy ").count(), 8);
        assert_eq!(config.matches("label = convolver").count(), 16);
        assert_eq!(config.matches("label = mixer").count(), 2);

        // Every entry, in order, name and channel — not a sample of them.
        assert_eq!(
            CONVOLVERS.len(),
            EXPECTED_CONVOLVERS.len(),
            "convolver count"
        );
        for (index, (name, channel)) in EXPECTED_CONVOLVERS.into_iter().enumerate() {
            assert_eq!(CONVOLVERS[index].name, name, "convolver {index} name");
            assert_eq!(
                CONVOLVERS[index].channel, channel,
                "convolver {index} ({name}) must read channel {channel}"
            );
        }
    }

    #[test]
    fn the_input_duplicator_table_matches_the_official_layout() {
        assert_eq!(COPIES, EXPECTED_COPIES);
        // The duplicators exist to feed the eight advertised surround channels.
        for (copy, position) in COPIES.iter().zip(INPUT_POSITIONS) {
            assert_eq!(copy.strip_prefix("copy"), Some(position));
        }
    }

    #[test]
    fn the_link_tables_match_the_official_layout() {
        assert_eq!(INPUT_LINKS, EXPECTED_INPUT_LINKS);
        assert_eq!(OUTPUT_LINKS, EXPECTED_OUTPUT_LINKS);

        // Every link must reference nodes the graph actually declares.
        for (output, input) in INPUT_LINKS {
            assert!(COPIES.contains(&output), "unknown link source {output}");
            assert!(
                CONVOLVERS.iter().any(|convolver| convolver.name == input),
                "unknown link target {input}"
            );
        }
        for (output, mixer, index) in OUTPUT_LINKS {
            assert!(
                CONVOLVERS.iter().any(|convolver| convolver.name == output),
                "unknown link source {output}"
            );
            assert!(matches!(mixer, "mixL" | "mixR"), "unknown mixer {mixer}");
            assert!((1..=8).contains(&index), "mixer slot {index} out of range");
        }

        // Each convolver is fed exactly once and mixed down exactly once.
        for convolver in &CONVOLVERS {
            assert_eq!(
                INPUT_LINKS
                    .iter()
                    .filter(|(_, input)| *input == convolver.name)
                    .count(),
                1,
                "{} must have exactly one input link",
                convolver.name
            );
            assert_eq!(
                OUTPUT_LINKS
                    .iter()
                    .filter(|(output, _, _)| *output == convolver.name)
                    .count(),
                1,
                "{} must have exactly one output link",
                convolver.name
            );
        }
    }

    #[test]
    fn the_rendered_nodes_and_links_blocks_match_the_tables_verbatim() {
        let config = render();
        assert!(
            config.contains(&expected_nodes_block(TEST_HRIR)),
            "rendered nodes block deviates from the official layout"
        );
        assert!(
            config.contains(&expected_links_block()),
            "rendered links block deviates from the official layout"
        );
    }

    #[test]
    fn fourteen_distinct_hrir_channels_are_used() {
        let mut channels: Vec<u8> = CONVOLVERS
            .iter()
            .map(|convolver| convolver.channel)
            .collect();
        channels.sort_unstable();
        channels.dedup();
        assert_eq!(channels, (0u8..=13).collect::<Vec<_>>());
    }

    #[test]
    fn paths_are_escaped() {
        let config = render_hrir("/weird/pa\"th\\dir/hrir.wav");
        assert!(config.contains("filename = \"/weird/pa\\\"th\\\\dir/hrir.wav\""));
        // The raw, unescaped quote must not appear mid-path.
        assert!(!config.contains("pa\"th"));
    }

    #[test]
    fn a_path_containing_template_markers_is_rendered_verbatim() {
        // A path that spells out every placeholder. Under a
        // substitute-then-substitute-again scheme these would be rewritten with
        // the node names, the whole nodes block, and so on.
        let path = "/hrir/{nodes}{links}{inputs}{positions}{sink}{output}{target}/hrir.wav";
        let config = render_hrir(path);

        assert!(
            config.contains(&format!("filename = \"{path}\"")),
            "the HRIR path was rewritten during rendering"
        );
        // Each marker survives exactly once per convolver node — that is, only
        // inside the path — and was never expanded.
        for marker in TEMPLATE_MARKERS {
            assert_eq!(
                config.matches(marker).count(),
                CONVOLVERS.len(),
                "marker {marker} was substituted inside the HRIR path"
            );
        }
        // Nothing the path spelled out leaked into the routing decision.
        assert_eq!(config.matches("target.object").count(), 1);
        assert!(config.contains(&format!("target.object = \"{TARGET_EQUALIZER_SINK}\"")));
        assert!(config.contains(&format!("node.name = \"{SPATIAL_SINK_NODE}\"")));
        assert!(config.contains(&format!("node.name = \"{SPATIAL_OUTPUT_NODE}\"")));
        // The rest of the graph is unaffected by the hostile path.
        assert!(config.contains(&expected_nodes_block(path)));
        assert!(config.contains(&expected_links_block()));
    }

    #[cfg(unix)]
    #[test]
    fn a_non_utf8_path_fails_closed_instead_of_being_rewritten() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        // Valid on a Unix filesystem, not valid UTF-8. `to_string_lossy` would
        // turn 0xFF into U+FFFD and name a different file.
        let path = PathBuf::from(OsStr::from_bytes(b"/hrir/\xffbroken/hrir.wav"));
        let rendered = render_filter_chain_config(&path);
        let error = rendered.expect_err("non-UTF-8 path must not render");

        assert!(
            error.contains("not valid UTF-8"),
            "unhelpful error: {error}"
        );
        // Actionable: it says what to do about it.
        assert!(error.contains("rename"), "unactionable error: {error}");
        // And it never silently produced the substituted path.
        assert!(!error.contains("filename ="));
    }

    #[test]
    fn structural_braces_are_preserved() {
        let config = render();
        assert!(config.contains("context.properties = { log.level = 0 }"));
        assert!(config.contains("filter.graph = {"));
        assert!(config.trim_end().ends_with(']'));
        // No unsubstituted placeholder markers remain.
        for marker in TEMPLATE_MARKERS {
            assert!(!config.contains(marker), "leftover marker {marker}");
        }
    }

    #[test]
    fn every_declared_marker_appears_in_the_template_exactly_once() {
        for marker in TEMPLATE_MARKERS {
            assert_eq!(
                TEMPLATE.matches(marker).count(),
                1,
                "marker {marker} must appear exactly once in the template"
            );
        }
    }

    #[test]
    fn no_brace_token_survives_rendering_unsubstituted() {
        // Catches a marker added to the template but not to the substitution
        // list, which `structural_braces_are_preserved` (driven by the declared
        // list) would miss. Every structural brace in the config is followed by
        // a space, so a `{lowercase-word}` can only be a leftover marker.
        let config = render();
        for (index, _) in config.match_indices('{') {
            let rest = &config[index + 1..];
            let word_len = rest
                .find(|character: char| !character.is_ascii_lowercase())
                .unwrap_or(rest.len());
            assert!(
                word_len == 0 || !rest[word_len..].starts_with('}'),
                "leftover marker {{{}}}",
                &rest[..word_len]
            );
        }
    }

    #[test]
    fn render_template_copies_unknown_braces_through() {
        assert_eq!(render_template("a { b {x} c", &[("{x}", "X")]), "a { b X c");
        // A substituted value is never re-scanned.
        let values = [("{x}", "{y}"), ("{y}", "!")];
        assert_eq!(render_template("{x}{y}", &values), "{y}!");
    }

    #[test]
    fn no_vendor_branding_leaks_into_the_config() {
        let config = render().to_ascii_lowercase();
        for banned in ["dts", "dolby", "atmos", "quantum"] {
            assert!(!config.contains(banned), "config leaked `{banned}`");
        }
    }
}
