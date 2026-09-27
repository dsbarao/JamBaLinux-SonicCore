#!/usr/bin/env python3
"""Offline ledger for passive JamBaLinux audio-gain snapshots.

This program deliberately reads only its supplied JSON document and writes a
report.  It never discovers, starts, or controls an audio service.
"""

import argparse
import json
import math
import sys
from pathlib import Path


DRY_COEFFICIENT = 0.707
DRY_CHANNELS = {
    "left": {"FL": 1.0, "FC": DRY_COEFFICIENT, "SL": DRY_COEFFICIENT, "RL": DRY_COEFFICIENT},
    "right": {"FR": 1.0, "FC": DRY_COEFFICIENT, "SR": DRY_COEFFICIENT, "RR": DRY_COEFFICIENT},
}
HEADROOM_DBFS = 0.0


def db(linear):
    return "-inf" if linear == 0 else round(20 * math.log10(linear), 3)


def observed(value):
    return value if value is not None else "indeterminado (não exposto no log)"


def values(value, expected=None):
    if not isinstance(value, list) or (expected is not None and len(value) != expected):
        return None
    if not all(isinstance(item, (int, float)) and not isinstance(item, bool) for item in value):
        return None
    return value


def format_summary(stage):
    if not isinstance(stage, dict):
        return "indeterminado (não exposto no log)"
    fields = ["format", "rate", "channels", "position"]
    present = {field: stage[field] for field in fields if stage.get(field) is not None}
    return present or "indeterminado (não exposto no log)"


def mode_from_gains(gains):
    if not isinstance(gains, dict):
        return "indeterminado (controles wet/dry ausentes)"
    wet = values(gains.get("wet"), 2)
    dry = values(gains.get("dry"), 2)
    if wet is None or dry is None:
        return "indeterminado (controles wet/dry incompletos)"
    if wet == [1, 1] and dry == [0, 0]:
        return "binaural nominal (wet=1, dry=0)"
    if wet == [0, 0] and dry == [1, 1]:
        return "bypass nominal (wet=0, dry=1)"
    return "transição ou estado não nominal (não inferir modo)"


def main():
    parser = argparse.ArgumentParser(description="Analyze a supplied passive audio-gain ledger offline.")
    parser.add_argument("input", type=Path, help="JSON ledger/fixture supplied by the caller")
    parser.add_argument("--pretty", action="store_true", help="indent JSON output")
    args = parser.parse_args()
    try:
        source = json.loads(args.input.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        parser.error("cannot read supplied JSON ledger: %s" % error)
    if not isinstance(source, dict) or source.get("schema") != "jambalinux-audio-gain-ledger/v1":
        parser.error("expected schema jambalinux-audio-gain-ledger/v1")

    spatial = source.get("spatial") if isinstance(source.get("spatial"), dict) else {}
    eq = source.get("eq") if isinstance(source.get("eq"), dict) else {}
    streams = source.get("streams") if isinstance(source.get("streams"), dict) else {}
    sinks = source.get("sinks") if isinstance(source.get("sinks"), dict) else {}
    measured = source.get("measured_peaks_dbfs") if isinstance(source.get("measured_peaks_dbfs"), dict) else {}
    dry_sum = round(sum(DRY_CHANNELS["left"].values()), 3)
    dry_sum_db = db(dry_sum)
    requested = values(eq.get("requested_bands_db"), 10)
    applied = values(eq.get("applied_bands_db"), 10)

    risks = []
    if dry_sum_db > HEADROOM_DBFS:
        risks.append({"kind": "dry-downmix-correlated-maximum", "value_dbfs": dry_sum_db,
                      "reason": "soma seca máxima teórica excede 0 dBFS para canais correlacionados"})
    for name, peak in measured.items():
        if isinstance(peak, (int, float)) and not isinstance(peak, bool) and peak > HEADROOM_DBFS:
            risks.append({"kind": "measured-peak", "stage": name, "value_dbfs": peak,
                          "reason": "pico medido excede 0 dBFS"})
    for name, bands in (("requested", requested), ("applied", applied)):
        if bands is not None and any(value > 0 for value in bands):
            risks.append({"kind": "positive-eq", "stage": name,
                          "reason": "EQ positivo sem preamp automático; clipping não foi excluído"})

    spatial_formats = spatial.get("formats") if isinstance(spatial.get("formats"), dict) else {}
    game = sinks.get("game") if isinstance(sinks.get("game"), dict) else {}
    chat = sinks.get("chat") if isinstance(sinks.get("chat"), dict) else {}
    report = {
        "schema": "jambalinux-audio-gain-report/v1",
        "source": str(args.input),
        "limits_derived": {
            "headroom_dbfs": HEADROOM_DBFS,
            "dry_downmix_coefficients": DRY_CHANNELS,
            "dry_downmix_maximum_linear": dry_sum,
            "dry_downmix_maximum_dbfs": dry_sum_db,
            "note": "limite para entradas correlacionadas em escala full-scale; não é pico medido",
        },
        "observed": {
            "stream_volumes": observed(streams or None),
            "sink_volumes": observed(sinks or None),
            "wet_dry_gains": observed(spatial.get("wet_dry_gains")),
            "wet_dry_state": mode_from_gains(spatial.get("wet_dry_gains")),
            "eq_requested_bands_db": observed(requested),
            "eq_applied_bands_db": observed(applied),
            "measured_peaks_dbfs": observed(measured or None),
            "formats": {
                "spatial_input": format_summary(spatial_formats.get("input")),
                "spatial_output": format_summary(spatial_formats.get("output")),
                "game": format_summary(game), "chat": format_summary(chat),
            },
        },
        "comparisons": {
            "game_vs_chat": {
                "game": format_summary(game), "chat": format_summary(chat),
                "format_or_rate_divergent": (None if not game or not chat else
                    any(game.get(key) != chat.get(key) for key in ("format", "rate", "channels", "position"))),
            }
        },
        "indeterminate": {
            "convolver_hrir_gain": "indeterminado (normalização/pico/RMS HRIR não expostos no log)",
            "endpoint_gain_or_spl": "indeterminado (volume do sink não mede SPL do headset)",
            "resampler_or_conversion_gain": "indeterminado (não exposto como ganho no log)",
            "game_chat_dial_effect": "indeterminado (posição HID não prova ganho, endpoint ou loudness)",
        },
        "clipping_risks": risks,
        "recommendation": "não aumentar volume acima de 100%; obter A/B digital pré/pós antes de qualquer correção",
    }
    json.dump(report, sys.stdout, ensure_ascii=False, indent=2 if args.pretty else None, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
