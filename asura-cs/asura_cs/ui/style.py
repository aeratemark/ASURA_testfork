"""Old-school look: Fusion with the classic Windows 2000 grey palette the TES Construction Set wore."""

from __future__ import annotations

from PySide6.QtGui import QColor, QFont, QPalette
from PySide6.QtWidgets import QApplication, QStyleFactory

GREY = "#d4d0c8"
DARK = "#808080"
SHADOW = "#404040"
HILIGHT = "#0a246a"

STYLESHEET = f"""
QMainWindow::separator {{ background: {GREY}; width: 4px; height: 4px; }}
QDockWidget {{ font-weight: normal; }}
QDockWidget::title {{ background: {HILIGHT}; color: white; padding: 2px 4px; text-align: left; }}
QToolBar {{ background: {GREY}; border: 1px solid {DARK}; spacing: 2px; padding: 1px; }}
QToolButton {{ border: 1px solid transparent; padding: 2px; }}
QToolButton:hover {{ border: 1px outset {DARK}; }}
QToolButton:checked, QToolButton:pressed {{ border: 1px inset {DARK}; background: #c0bcb4; }}
QTreeWidget, QListWidget, QTableWidget, QPlainTextEdit, QLineEdit, QTextEdit {{
    background: white; border: 2px inset {DARK}; selection-background-color: {HILIGHT};
    selection-color: white; }}
QGroupBox {{ border: 1px groove {DARK}; margin-top: 8px; padding-top: 4px; }}
QGroupBox::title {{ subcontrol-origin: margin; left: 6px; padding: 0 2px; }}
QStatusBar {{ border-top: 1px solid {DARK}; }}
QStatusBar QLabel {{ border: 1px inset {DARK}; padding: 0 4px; }}
QHeaderView::section {{ background: {GREY}; border: 1px outset {DARK}; padding: 1px 4px; }}
QTabBar::tab {{ background: {GREY}; border: 1px outset {DARK}; padding: 2px 8px; }}
QTabBar::tab:selected {{ border-bottom: none; background: #e4e0d8; }}
"""


def apply(app: QApplication) -> None:
    app.setStyle(QStyleFactory.create("Fusion"))
    pal = QPalette()
    grey = QColor(GREY)
    pal.setColor(QPalette.Window, grey)
    pal.setColor(QPalette.Button, grey)
    pal.setColor(QPalette.Base, QColor("white"))
    pal.setColor(QPalette.AlternateBase, QColor("#f0eee8"))
    pal.setColor(QPalette.Text, QColor("black"))
    pal.setColor(QPalette.WindowText, QColor("black"))
    pal.setColor(QPalette.ButtonText, QColor("black"))
    pal.setColor(QPalette.Highlight, QColor(HILIGHT))
    pal.setColor(QPalette.HighlightedText, QColor("white"))
    pal.setColor(QPalette.Mid, QColor(DARK))
    pal.setColor(QPalette.Dark, QColor(SHADOW))
    pal.setColor(QPalette.ToolTipBase, QColor("#ffffe1"))
    pal.setColor(QPalette.ToolTipText, QColor("black"))
    app.setPalette(pal)
    font = QFont("Tahoma", 8)
    font.setStyleHint(QFont.SansSerif)
    app.setFont(font)
    app.setStyleSheet(STYLESHEET)
