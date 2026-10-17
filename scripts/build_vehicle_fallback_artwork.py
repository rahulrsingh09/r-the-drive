#!/usr/bin/env python3
"""Build normalized, transparent fallback artwork for Panda vehicle surfaces.

The source renderings in ``assets/vehicles_generated`` are intentionally kept as
received. This script removes their flat white matte and composes four stable
presentation canvases per model:

* overview: portrait overhead artwork that matches the API rotation contract
* charging: a full side profile composed for the charging connection chip
* health: a normalized three-quarter hero composition
* vehicle-card: a compact full side profile for vehicle lists and pickers

The output is deterministic and includes a machine-readable manifest plus an
optional contact sheet for review.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import shutil
import tempfile
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable, Literal

from PIL import Image, ImageChops, ImageDraw, ImageFilter, ImageFont

MODEL_NAMES = ("r1s", "r1t", "r2s")
Usage = Literal["overview", "charging", "health", "vehicle-card"]

SOURCE_BY_USAGE: dict[Usage, str] = {
    "overview": "top.png",
    "charging": "charge.png",
    "health": "three_quarter.png",
    "vehicle-card": "side.png",
}

CANVAS_BY_USAGE: dict[Usage, tuple[int, int]] = {
    "overview": (640, 1440),
    "charging": (1200, 900),
    "health": (1600, 900),
    "vehicle-card": (1200, 560),
}

# Charging fallbacks use the full side profile so the vehicle is not cut at the
# canvas edge. The live API artwork keeps its own charge-port crop behavior.
CHARGING_PORT_FRACTION: dict[str, float] = {
    "r1s": 1.00,
    "r1t": 1.00,
    "r2s": 1.00,
}

FULL_PROFILE_CHARGING_MODELS = frozenset({"r1s", "r1t", "r2s"})


@dataclass(frozen=True)
class AssetReport:
    model: str
    usage: Usage
    source: str
    output: str
    width: int
    height: int
    alpha_coverage: float
    visible_bbox: tuple[int, int, int, int] | None
    sha256: str


COMPOSITIONS = {
    "overview": (0.012, 0.86, 0.90, 0.50, 0.95),
    "health": (0.018, 0.90, 0.82, 0.52, 0.95),
    "vehicle-card": (0.012, 0.94, 0.82, 0.50, 0.91),
}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    specifications = {
        "root": (Path("assets/vehicles_generated"), "Directory containing model source folders."),
        "output": (Path("apps/web/public/vehicle-images/fallbacks"), "Directory for generated WebP artwork and manifest."),
        "preview": (None, "Optional contact-sheet path."),
    }
    for name, (default, help_text) in specifications.items():
        parser.add_argument("--" + name, type=Path, default=default, help=help_text)
    parser.add_argument("--check", action="store_true",
                        help="Regenerate in a temporary directory and fail when checked-in output differs.")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    root, output = args.root.resolve(), args.output.resolve()
    if not args.check:
        reports = build_all(root, output)
        write_manifest(output, reports)
        if args.preview is not None:
            write_contact_sheet(output, reports, args.preview.resolve())
        print(f"Generated {len(reports)} normalized fallback assets in {output}")
        return 0
    with tempfile.TemporaryDirectory(prefix="vehicle-artwork-") as directory:
        candidate = Path(directory) / "fallbacks"
        reports = build_all(root, candidate)
        write_manifest(candidate, reports)
        differences = compare_directories(candidate, output)
    if not differences:
        print(f"Vehicle fallback artwork is current ({len(reports)} assets).")
        return 0
    print("Vehicle fallback artwork is out of date:")
    print("\n".join("  - " + difference for difference in differences))
    print("Run scripts/build_vehicle_fallback_artwork.py and commit the generated files.")
    return 1


def build_all(root: Path, output: Path) -> list[AssetReport]:
    validate_sources(root)
    tasks = [(model, usage, root / model / source,
              output / model / (("side" if usage == "vehicle-card" else usage) + ".webp"))
             for model in MODEL_NAMES for usage, source in SOURCE_BY_USAGE.items()]
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True, exist_ok=True)
    reports = []
    for model, usage, source, destination in tasks:
        destination.parent.mkdir(parents=True, exist_ok=True)
        with Image.open(source) as image:
            composed = compose_usage(remove_white_matte(image), model=model, usage=usage)
        composed.save(destination, "WEBP", lossless=True, method=6, exact=True)
        reports.append(report_asset(root, output, model, usage, source, destination, composed))
    return reports


def validate_sources(root: Path) -> None:
    absent = []
    for model in MODEL_NAMES:
        for source in SOURCE_BY_USAGE.values():
            path = root / model / source
            if not path.is_file():
                absent.append("  - " + str(path))
    if absent:
        raise SystemExit("Missing vehicle artwork sources:\n" + "\n".join(absent))


def remove_white_matte(source: Image.Image) -> Image.Image:
    rgb = source.convert("RGB")
    mask_source = rgb.convert("RGBA")
    width, height = rgb.size
    border = {(x, y) for x in (0, width - 1) for y in (0, height - 1)}
    border.update(((width // 2, 0), (width // 2, height - 1), (0, height // 2), (width - 1, height // 2)))
    for point in border:
        ImageDraw.floodfill(mask_source, point, (255, 255, 255, 0), thresh=30)
    alpha = mask_source.getchannel("A")
    transparent = alpha.point([255] + [0] * 255).filter(ImageFilter.MaxFilter(11))
    # The darkest channel measures distance from the white matte.
    red, green, blue = rgb.split()
    darkness = ImageChops.invert(ImageChops.darker(red, ImageChops.darker(green, blue)))
    softened = darkness.point([_soft_alpha_from_darkness(value) for value in range(256)])
    mask_source = rgb.convert("RGBA")
    mask_source.putalpha(Image.composite(softened, alpha, transparent))
    return decontaminate_white_fringe(mask_source)


def _soft_alpha_from_darkness(value: int) -> int:
    fraction = min(1.0, max(0.0, (value - 3) / 31))
    return round((fraction * fraction * (3.0 - 2.0 * fraction)) * 255)


def decontaminate_white_fringe(image: Image.Image) -> Image.Image:
    corrected = []
    for pixel in image.getdata():
        alpha = pixel[3]
        if alpha == 255:
            corrected.append(pixel)
        elif alpha == 0:
            corrected.append((0, 0, 0, 0))
        else:
            corrected.append(tuple(_unblend_white(channel, alpha / 255.0) for channel in pixel[:3]) + (alpha,))
    image.putdata(corrected)
    return image


def _unblend_white(channel: int, opacity: float) -> int:
    if opacity > 0:
        estimate = round((channel - 255.0 * (1.0 - opacity)) / opacity)
        return sorted((0, estimate, 255))[1]
    return 0


def compose_usage(image: Image.Image, *, model: str, usage: Usage) -> Image.Image:
    if usage == "charging":
        return compose_charging(image, model=model)
    if usage not in COMPOSITIONS:
        raise AssertionError(f"Unsupported usage: {usage}")
    padding, width, height, center, bottom = COMPOSITIONS[usage]
    return place_on_canvas(crop_visible(image, padding_fraction=padding), CANVAS_BY_USAGE[usage],
                           max_width_fraction=width, max_height_fraction=height,
                           center_x_fraction=center, bottom_fraction=bottom)


def compose_overview(image: Image.Image) -> Image.Image:
    return compose_usage(image, model="", usage="overview")


def compose_health(image: Image.Image) -> Image.Image:
    return compose_usage(image, model="", usage="health")


def compose_vehicle_card(image: Image.Image) -> Image.Image:
    return compose_usage(image, model="", usage="vehicle-card")


def padded_crop(image, bounds, padding):
    left, top, right, bottom = bounds
    px, py = padding
    return image.crop((max(0, left - px), max(0, top - py),
                       min(image.width, right + px), min(image.height, bottom + py)))


def composite_at(content, canvas_size, size, position):
    resized = content.resize(size, Image.Resampling.LANCZOS)
    result = Image.new("RGBA", canvas_size, (0, 0, 0, 0))
    result.alpha_composite(resized, position)
    return result


def compose_charging(image: Image.Image, *, model: str) -> Image.Image:
    bounds = alpha_bbox(image)
    if bounds is None:
        raise ValueError(f"Charging source for {model} has no visible pixels")
    left, top, right, bottom = bounds
    width, height = right - left, bottom - top
    edge = min(image.width, math.ceil(left + width * CHARGING_PORT_FRACTION[model]))
    content = padded_crop(image, (left, top, edge, bottom),
                          (max(8, round(width * 0.012)), max(8, round(height * 0.035))))
    cw, ch = CANVAS_BY_USAGE["charging"]
    full = model in FULL_PROFILE_CHARGING_MODELS
    ratios = [round(ch * (0.78 if full else 0.92)) / content.height]
    if full:
        ratios.append(round(cw * 0.96) / content.width)
    scale = min(ratios)
    size = (round(content.width * scale), round(content.height * scale))
    position = (cw - size[0] if full else round(cw * 0.33), round(ch * 0.96) - size[1])
    return composite_at(content, (cw, ch), size, position)


def crop_visible(image: Image.Image, *, padding_fraction: float) -> Image.Image:
    bounds = alpha_bbox(image)
    if bounds is None:
        raise ValueError("Artwork has no visible pixels after matte removal")
    dimensions = (bounds[2] - bounds[0], bounds[3] - bounds[1])
    padding = tuple(max(4, round(length * padding_fraction)) for length in dimensions)
    return padded_crop(image, bounds, padding)


def place_on_canvas(content: Image.Image, canvas_size: tuple[int, int], *,
                    max_width_fraction: float, max_height_fraction: float,
                    center_x_fraction: float, bottom_fraction: float) -> Image.Image:
    fractions = (max_width_fraction, max_height_fraction)
    scale = min(canvas * fraction / length for canvas, fraction, length in zip(canvas_size, fractions, content.size))
    size = tuple(max(1, round(length * scale)) for length in content.size)
    anchor = (round(canvas_size[0] * center_x_fraction), round(canvas_size[1] * bottom_fraction))
    return composite_at(content, canvas_size, size, (anchor[0] - size[0] // 2, anchor[1] - size[1]))


def alpha_bbox(image: Image.Image, threshold: int = 6) -> tuple[int, int, int, int] | None:
    lookup = [int(level > threshold) * 255 for level in range(256)]
    return image.getchannel("A").point(lookup).getbbox()


def report_asset(root: Path, output_root: Path, model: str, usage: Usage,
                 source_path: Path, output_path: Path, image: Image.Image) -> AssetReport:
    counts = image.getchannel("A").histogram()
    total = sum(level * count for level, count in enumerate(counts))
    dimensions = dict(zip(("width", "height"), image.size))
    return AssetReport(model={"r2s": "R2"}.get(model, model.upper()), usage=usage,
                       source=source_path.relative_to(root.parent).as_posix(),
                       output=output_path.relative_to(output_root).as_posix(), **dimensions,
                       alpha_coverage=round(total / (255 * image.width * image.height), 6),
                       visible_bbox=alpha_bbox(image), sha256=hashlib.sha256(output_path.read_bytes()).hexdigest())


def write_manifest(output: Path, reports: Iterable[AssetReport]) -> None:
    assets = list(map(asdict, reports))
    metadata = dict(version=1, background="transparent", format="lossless-webp", assets=assets)
    (output / "manifest.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")


def compare_directories(generated: Path, checked_in: Path) -> list[str]:
    def contents(directory):
        return {path.relative_to(directory).as_posix(): hashlib.sha256(path.read_bytes()).digest()
                for path in directory.rglob("*") if path.is_file()}
    expected, actual = contents(generated), contents(checked_in)
    result = []
    for name in sorted(set(expected).union(actual)):
        if name not in actual:
            kind = "missing checked-in file"
        elif name not in expected:
            kind = "unexpected checked-in file"
        elif actual[name] != expected[name]:
            kind = "content differs"
        else:
            continue
        result.append(kind + ": " + name)
    return result


def write_contact_sheet(output: Path, reports: list[AssetReport], preview_path: Path) -> None:
    sheet = checkerboard((1680, math.ceil(len(reports) / 3) * 444), 24)
    draw, font = ImageDraw.Draw(sheet), ImageFont.load_default()
    for index, report in enumerate(reports):
        y, x = (index // 3) * 444, (index % 3) * 560
        with Image.open(output / report.output) as source:
            preview = source.convert("RGBA")
        preview.thumbnail((532, 362), Image.Resampling.LANCZOS)
        sheet.alpha_composite(preview, (x + (560 - preview.width) // 2, y + (390 - preview.height) // 2))
        draw.rectangle((x, y + 390, x + 560, y + 444), fill=(246, 246, 246, 255))
        text = f"{report.model} · {report.usage} · {report.width}×{report.height} · alpha {report.alpha_coverage:.3f}"
        for offset, label, color in ((400, text, 20), (419, report.output, 70)):
            draw.text((x + 10, y + offset), label, fill=(color, color, color, 255), font=font)
    preview_path.parent.mkdir(parents=True, exist_ok=True)
    sheet.convert("RGB").save(preview_path, "PNG", optimize=True)


def checkerboard(size: tuple[int, int], cell: int) -> Image.Image:
    result = Image.new("RGBA", size, (244, 244, 244, 255))
    draw = ImageDraw.Draw(result)
    for row, y in enumerate(range(0, size[1], cell)):
        for x in range(cell if row % 2 == 0 else 0, size[0], 2 * cell):
            draw.rectangle((x, y, x + cell - 1, y + cell - 1), fill=(222, 222, 222, 255))
    return result


if __name__ == "__main__":
    raise SystemExit(main())
