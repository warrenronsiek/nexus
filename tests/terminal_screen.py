# @feature observability-ui
# @spec docs/features/observability-ui.md
# @boundary renderer-vt-output: bounded current-screen projection for the pinned renderer's cursor, erase, style and hyperlink sequences.
"""Track the pinned alternate-screen renderer, without keeping a transcript."""
from __future__ import annotations

import codecs
import unicodedata


class TerminalScreen:
    def __init__(self, rows: int, columns: int) -> None:
        self.rows = rows
        self.columns = columns
        self.cells = [[" "] * columns for _ in range(rows)]
        self.row = 0
        self.column = 0
        self.mode = "text"
        self.parameters = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")

    def feed(self, output: bytes) -> None:
        for character in self.decoder.decode(output):
            if self.mode == "text":
                self.write(character)
            elif self.mode == "escape":
                self.mode = {"[": "csi", "]": "osc"}.get(character, "text")
                self.parameters = ""
            elif self.mode == "csi":
                self.csi(character)
            elif self.mode == "osc":
                if character == "\x07":
                    self.mode = "text"
                elif character == "\x1b":
                    self.mode = "osc_escape"
            elif self.mode == "osc_escape":
                self.mode = "text" if character == "\\" else "osc"

    def write(self, character: str) -> None:
        if character == "\x1b":
            self.mode = "escape"
        elif character == "\r":
            self.column = 0
        elif character == "\n":
            self.row = min(self.rows - 1, self.row + 1)
        elif character >= " " and character != "\x7f":
            self.printable(character)

    def printable(self, character: str) -> None:
        if unicodedata.combining(character):
            if self.column:
                self.cells[self.row][min(self.column, self.columns) - 1] += character
            return
        width = 2 if unicodedata.east_asian_width(character) in ("W", "F") else 1
        if self.column + width <= self.columns:
            self.cells[self.row][self.column] = character
            if width == 2:
                self.cells[self.row][self.column + 1] = ""
        self.column = min(self.columns, self.column + width)

    def csi(self, character: str) -> None:
        if "@" <= character <= "~":
            self.command(character)
            self.mode = "text"
        else:
            assert len(self.parameters) < 64, "oversized renderer CSI sequence"
            self.parameters += character

    def command(self, command: str) -> None:
        # Styles, synchronized output, mouse/cursor modes and capability queries
        # do not alter cells. The pinned renderer positions rows absolutely.
        if command not in ("H", "f", "J", "K"):
            return
        values = [int(value or "0") for value in self.parameters.split(";")]
        if command in ("H", "f"):
            self.row = min(self.rows - 1, max(1, values[0]) - 1)
            self.column = min(self.columns - 1, max(1, values[1] if len(values) > 1 else 1) - 1)
        elif command == "K":
            self.erase_line(values[0])
        elif values[0] == 2:
            self.cells = [[" "] * self.columns for _ in range(self.rows)]
        elif values[0] == 0:
            self.erase_line(0)
            for row in range(self.row + 1, self.rows):
                self.cells[row] = [" "] * self.columns

    def erase_line(self, mode: int) -> None:
        start = 0 if mode in (1, 2) else self.column
        end = self.columns if mode in (0, 2) else self.column + 1
        self.cells[self.row][start:end] = [" "] * (end - start)

    def text(self) -> str:
        return "\n".join("".join(line).rstrip() for line in self.cells)
