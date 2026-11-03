"""Lua syntax highlighting for QPlainTextEdit documents."""

from __future__ import annotations

import re

from PySide6.QtGui import QColor, QFont, QSyntaxHighlighter, QTextCharFormat

KEYWORDS = ("and break do else elseif end false for function goto if in local nil not or repeat return then "
            "true until while").split()
API_ROOTS = ("world", "quest", "character", "dialogue", "self", "npc", "area")


def _fmt(color: str, bold: bool = False, italic: bool = False) -> QTextCharFormat:
    f = QTextCharFormat()
    f.setForeground(QColor(color))
    if bold:
        f.setFontWeight(QFont.Bold)
    f.setFontItalic(italic)
    return f


class LuaHighlighter(QSyntaxHighlighter):
    def __init__(self, document):
        super().__init__(document)
        self.rules = [
            (re.compile(r"\b(" + "|".join(KEYWORDS) + r")\b"), _fmt("#00007f", bold=True)),
            (re.compile(r"\b(" + "|".join(API_ROOTS) + r")\b"), _fmt("#7f007f", bold=True)),
            (re.compile(r"\b\d+(\.\d+)?\b|\b0x[0-9a-fA-F]+\b"), _fmt("#7f3f00")),
            (re.compile(r"\"(\\.|[^\"\\])*\"|'(\\.|[^'\\])*'"), _fmt("#007f00")),
            (re.compile(r"--(?!\[\[).*$"), _fmt("#808080", italic=True)),
        ]
        self.block_comment = _fmt("#808080", italic=True)

    def highlightBlock(self, text: str) -> None:  # noqa: N802 - Qt API
        for rx, fmt in self.rules:
            for m in rx.finditer(text):
                self.setFormat(m.start(), m.end() - m.start(), fmt)
        # --[[ ... ]] block comments across lines
        self.setCurrentBlockState(0)
        start = 0 if self.previousBlockState() == 1 else text.find("--[[")
        while start >= 0:
            end = text.find("]]", start + (0 if self.previousBlockState() == 1 and start == 0 else 4))
            if end < 0:
                self.setCurrentBlockState(1)
                self.setFormat(start, len(text) - start, self.block_comment)
                break
            self.setFormat(start, end + 2 - start, self.block_comment)
            start = text.find("--[[", end + 2)
