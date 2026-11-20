//! Read-only summary of the Game audio path for the widget:
//! apps -> spatial graph -> equalizer -> Quantum Game sink.
//!
//! One `pw-dump` snapshot is enough: node presence, PipeWire's own node
//! state (`running` means audio is flowing through it) and the links between
//! the nodes. Nothing is linked, moved or changed.

use std::collections::HashMap;

use serde_json::{Value, json};

use crate::spatial::graph::{SPATIAL_OUTPUT_NODE, SPATIAL_SINK_NODE, TARGET_EQUALIZER_SINK};

const EQUALIZER_OUTPUT_NODE: &str = "jambalinux-soniccore-game-equalizer-output";

struct Node<'a> {
    name: &'a str,
    class: &'a str,
    running: bool,
}

fn nodes(objects: &[Value]) -> HashMap<u64, Node<'_>> {
    objects
        .iter()
        .filter(|object| {
            object.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Node")
        })
        .filter_map(|object| {
            let id = object.get("id")?.as_u64()?;
            let info = object.get("info")?;
            let props = info.get("props")?;
            Some((
                id,
                Node {
                    name: props.get("node.name").and_then(Value::as_str).unwrap_or(""),
                    class: props
                        .get("media.class")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    running: info.get("state").and_then(Value::as_str) == Some("running"),
                },
            ))
        })
        .collect()
}

/// `(output node id, input node id)` of every link.
fn links(objects: &[Value]) -> Vec<(u64, u64)> {
    objects
        .iter()
        .filter(|object| {
            object.get("type").and_then(Value::as_str) == Some("PipeWire:Interface:Link")
        })
        .filter_map(|object| {
            let info = object.get("info")?;
            Some((
                info.get("output-node-id")?.as_u64()?,
                info.get("input-node-id")?.as_u64()?,
            ))
        })
        .collect()
}

fn is_internal(name: &str) -> bool {
    name == SPATIAL_OUTPUT_NODE
        || name == EQUALIZER_OUTPUT_NODE
        || name.starts_with("jambalinux-soniccore-")
}

/// Builds the path summary from one `pw-dump` snapshot. `game_sink` is the
/// exact Quantum Game sink name, or `None` when the headset is absent.
pub fn summarize(objects: &[Value], game_sink: Option<&str>) -> Value {
    let nodes = nodes(objects);
    let links = links(objects);
    let find = |name: &str| {
        nodes
            .iter()
            .find(|(_, node)| node.name == name)
            .map(|(id, node)| (*id, node))
    };
    let spatial = find(SPATIAL_SINK_NODE);
    let spatial_output = find(SPATIAL_OUTPUT_NODE);
    let equalizer = find(TARGET_EQUALIZER_SINK);
    let equalizer_output = find(EQUALIZER_OUTPUT_NODE);
    let headset = game_sink.and_then(find);

    let linked = |from: Option<(u64, &Node)>, to: Option<(u64, &Node)>| match (from, to) {
        (Some((from, _)), Some((to, _))) => links.contains(&(from, to)),
        _ => false,
    };

    // Application playback streams feeding any stage of the Game path.
    let app_streams = nodes
        .iter()
        .filter(|(_, node)| node.class == "Stream/Output/Audio" && !is_internal(node.name))
        .collect::<Vec<_>>();
    let feeding = |sink: Option<(u64, &Node)>| {
        sink.is_some_and(|(sink, _)| {
            app_streams
                .iter()
                .any(|(stream, _)| links.contains(&(**stream, sink)))
        })
    };
    let into_game_path = app_streams
        .iter()
        .filter(|(stream, _)| {
            [spatial, equalizer, headset]
                .into_iter()
                .flatten()
                .any(|(sink, _)| links.contains(&(**stream, sink)))
        })
        .collect::<Vec<_>>();

    let stage = |node: Option<(u64, &Node)>| {
        json!({
            "present": node.is_some(),
            "running": node.is_some_and(|(_, node)| node.running),
        })
    };

    json!({
        "schema": 1,
        "stages": {
            "streams": {
                "count": into_game_path.len(),
                "running": into_game_path.iter().any(|(_, node)| node.running),
            },
            "spatial": stage(spatial),
            "equalizer": stage(equalizer),
            "headset": stage(headset),
        },
        "links": {
            "streams_spatial": feeding(spatial),
            "streams_equalizer": feeding(equalizer),
            "streams_headset": feeding(headset),
            "spatial_equalizer": linked(spatial_output, equalizer),
            "equalizer_headset": linked(equalizer_output, headset),
        },
    })
}

/// `soniccore audio-path --format json`
pub fn print_json() -> Result<(), String> {
    let objects = crate::pipewire::pw_dump()?;
    let game = crate::pipewire::game_sink_name(&objects).ok();
    let summary = summarize(&objects, game.as_deref());
    println!(
        "{}",
        serde_json::to_string_pretty(&summary).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GAME: &str = "alsa_output.quantum.game";

    fn node(id: u64, name: &str, class: &str, state: &str) -> Value {
        json!({ "id": id, "type": "PipeWire:Interface:Node",
                "info": { "state": state, "props": { "node.name": name, "media.class": class } } })
    }

    fn link(from: u64, to: u64) -> Value {
        json!({ "type": "PipeWire:Interface:Link",
                "info": { "output-node-id": from, "input-node-id": to } })
    }

    fn full_path(app_state: &str) -> Vec<Value> {
        vec![
            node(1, "Google Chrome", "Stream/Output/Audio", app_state),
            node(2, SPATIAL_SINK_NODE, "Audio/Sink", "running"),
            node(3, SPATIAL_OUTPUT_NODE, "Stream/Output/Audio", "running"),
            node(4, TARGET_EQUALIZER_SINK, "Audio/Sink", "running"),
            node(5, EQUALIZER_OUTPUT_NODE, "Stream/Output/Audio", "running"),
            node(6, GAME, "Audio/Sink", "running"),
            node(7, "Firefox", "Stream/Output/Audio", "running"),
            link(1, 2),
            link(1, 2),
            link(3, 4),
            link(5, 6),
        ]
    }

    #[test]
    fn a_stream_through_spatial_lights_the_whole_path() {
        let summary = summarize(&full_path("running"), Some(GAME));
        assert_eq!(
            summary["stages"]["streams"]["count"], 1,
            "Firefox is not on the Game path"
        );
        assert_eq!(summary["stages"]["streams"]["running"], true);
        for stage in ["spatial", "equalizer", "headset"] {
            assert_eq!(summary["stages"][stage]["present"], true, "{stage}");
        }
        assert_eq!(summary["links"]["streams_spatial"], true);
        assert_eq!(summary["links"]["streams_equalizer"], false);
        assert_eq!(summary["links"]["spatial_equalizer"], true);
        assert_eq!(summary["links"]["equalizer_headset"], true);
    }

    #[test]
    fn a_paused_stream_and_a_broken_link_are_visible() {
        let mut objects = full_path("paused");
        objects.retain(|object| object != &link(3, 4));
        let summary = summarize(&objects, Some(GAME));
        assert_eq!(summary["stages"]["streams"]["running"], false);
        assert_eq!(summary["links"]["spatial_equalizer"], false);
    }

    #[test]
    fn without_the_headset_or_graphs_everything_reports_absent() {
        let summary = summarize(
            &[node(1, "Google Chrome", "Stream/Output/Audio", "running")],
            None,
        );
        assert_eq!(summary["stages"]["headset"]["present"], false);
        assert_eq!(summary["stages"]["spatial"]["present"], false);
        assert_eq!(summary["stages"]["streams"]["count"], 0);
        assert_eq!(summary["links"]["equalizer_headset"], false);
    }
}
