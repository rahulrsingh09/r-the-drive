"""Small synthetic image contracts; no vehicle source assets are modified."""
import tempfile
import unittest
from pathlib import Path
from PIL import Image, ImageDraw
import build_vehicle_fallback_artwork as artwork


class ArtworkContracts(unittest.TestCase):
    def test_matte_removes_exterior_but_preserves_enclosed_white_detail(self):
        image = Image.new('RGB', (64, 64), 'white')
        draw = ImageDraw.Draw(image)
        draw.rectangle((12, 12, 51, 51), fill=(20, 40, 60))
        draw.rectangle((25, 25, 38, 38), fill='white')
        clean = artwork.remove_white_matte(image)
        self.assertEqual(clean.getpixel((0, 0)), (0, 0, 0, 0))
        self.assertEqual(clean.getpixel((30, 30)), (255, 255, 255, 255))
        self.assertEqual(artwork.alpha_bbox(clean), (12, 12, 52, 52))

    def test_alpha_curve_boundary_monotonicity_and_white_unblend(self):
        curve = [artwork._soft_alpha_from_darkness(value) for value in range(256)]
        self.assertEqual(curve[:4], [0] * 4)
        self.assertEqual(curve[34:], [255] * 222)
        self.assertEqual(curve, sorted(curve))
        self.assertEqual(artwork._unblend_white(128, 0.5), 1)
        self.assertEqual(artwork._unblend_white(1, 0), 0)
        self.assertEqual(artwork._unblend_white(255, 0.01), 255)

    def test_empty_artwork_rejected_and_compositions_have_fixed_dimensions(self):
        empty = Image.new('RGBA', (64, 64))
        with self.assertRaisesRegex(ValueError, 'no visible'):
            artwork.crop_visible(empty, padding_fraction=0.1)
        with self.assertRaisesRegex(ValueError, 'no visible'):
            artwork.compose_charging(empty, model='r1s')
        image = Image.new('RGBA', (64, 32), (20, 40, 60, 255))
        for usage, size in artwork.CANVAS_BY_USAGE.items():
            self.assertEqual(artwork.compose_usage(image, model='r1s', usage=usage).size, size)
        with self.assertRaisesRegex(AssertionError, 'Unsupported usage'):
            artwork.compose_usage(image, model='r1s', usage='invalid')

    def test_directory_comparison_reports_added_removed_and_changed_files(self):
        with tempfile.TemporaryDirectory() as directory:
            first, second = Path(directory) / 'first', Path(directory) / 'second'
            first.mkdir(); second.mkdir()
            for root, name, value in [(first, 'a', 'expected'), (second, 'a', 'changed'),
                                      (first, 'b', 'missing'), (second, 'c', 'extra')]:
                (root / name).write_text(value)
            self.assertEqual(artwork.compare_directories(first, second), [
                'content differs: a', 'missing checked-in file: b', 'unexpected checked-in file: c'])


if __name__ == '__main__':
    unittest.main()
