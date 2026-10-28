# @feature observability-ui
# @spec docs/features/observability-ui.md
# @boundary terminal-replay-fixtures: controlled VT output reproduces incremental PTY rendering without wall-clock sleeps.
from collections import deque
from contextlib import contextmanager
from itertools import count
import subprocess
from typing import Iterator, cast
import unittest
from unittest.mock import Mock, patch

from tests.terminal_screen import TerminalScreen
from tests.tui_terminal import wait_for_text


@contextmanager
def replay(chunks: list[bytes]) -> Iterator[tuple[subprocess.Popen[bytes], int]]:
    process = cast(subprocess.Popen[bytes], Mock(spec=subprocess.Popen))
    pending = deque(chunks)
    ticks = count()
    with patch.object(process, "poll", return_value=None), \
         patch("tests.tui_terminal.time.monotonic", side_effect=lambda: next(ticks) / 5), \
         patch("tests.tui_terminal.select.select", return_value=([91], [], [])), \
         patch("tests.tui_terminal.os.read", side_effect=lambda *_: pending.popleft() if pending else b""):
        yield process, 91


class TerminalAssertionTests(unittest.TestCase):
    def test_header_only_redraw_keeps_the_visible_remedy_available_to_later_assertions(self) -> None:
        chunks = [
            b"\x1b[1;1Hloading\x1b[3;1HTimeline\x1b[4;1HSerialize native completion"
            b"\x1b[5;1HDuplicate native callbacks\x1b[6;1Hsrc/native-router.rs dirty true",
            b"\x1b[1;1H\x1b[2Klive",
        ]
        with replay(chunks) as (process, master):
            wait_for_text(process, master, (b"Timeline", b"Serialize native completion"), 1)
            wait_for_text(process, master, (b"live", b"Serialize native completion", b"Duplicate native callbacks", b"src/native-router.rs", b"dirty true"), 1)

    def test_erased_text_cannot_satisfy_a_later_assertion(self) -> None:
        with replay([b"Old concern\x1b[H\x1b[2KFresh view"]) as (process, master):
            with self.assertRaisesRegex(AssertionError, "Missing"):
                wait_for_text(process, master, b"Old concern", 1)


class TerminalScreenTests(unittest.TestCase):
    def test_cursor_overwrite_and_line_erasure_remove_old_cells(self) -> None:
        screen = TerminalScreen(3, 20)
        screen.feed(b"Old concern\x1b[HNew")
        self.assertIn("New concern", screen.text())
        self.assertNotIn("Old concern", screen.text())
        screen.feed(b"\x1b[1;4H\x1b[K")
        self.assertEqual(screen.text(), "New\n\n")
        screen.feed(b"\x1b[1;2H\x1b[1K")
        self.assertEqual(screen.text(), "  w\n\n")
        screen.feed(b"\x1b[2K")
        self.assertEqual(screen.text(), "\n\n")

    def test_display_erasure_drops_old_views_and_does_not_grow_a_transcript(self) -> None:
        screen = TerminalScreen(3, 20)
        screen.feed(b"Old view\x1b[2;1HOld metadata\x1b[2J")
        self.assertNotIn("Old", screen.text())
        for _ in range(1000):
            screen.feed(b"\x1b[H\x1b[2Klive")
        self.assertEqual(screen.text(), "live\n\n")
        self.assertEqual([len(row) for row in screen.cells], [20, 20, 20])

    def test_split_utf8_csi_and_osc_sequences_preserve_only_visible_cells(self) -> None:
        screen = TerminalScreen(3, 20)
        output = "\x1b[2;1H\x1b[37mTime ↓ 界\x1b[0m\x1b]8;;https://hidden\x07link\x1b]8;;\x1b\\".encode()
        for value in output:
            screen.feed(bytes([value]))
        self.assertEqual(screen.text(), "\nTime ↓ 界link\n")
        self.assertNotIn("hidden", screen.text())


if __name__ == "__main__":
    unittest.main()
