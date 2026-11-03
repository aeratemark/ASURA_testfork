"""A permissive stand-in for PySide6, used ONLY by tests/test_ui_smoke.py when the real PySide6 is
missing. It exercises the editor's own Python logic (wiring, signals, commands, panel rebuilds); it
proves nothing about Qt API correctness. Signals, QUndoStack and QTimer.singleShot behave for real.
"""

import sys
import types


class _Meta(type):
    def __getattr__(cls, name):
        if name.startswith("__"):
            raise AttributeError(name)
        return _Stub()

    def __or__(cls, other):
        return _Stub()


class _Stub(metaclass=_Meta):
    def __init__(self, *a, **k):
        pass

    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)
        return _Stub()

    def __call__(self, *a, **k):
        return _Stub()

    def __bool__(self):
        return True

    def __getitem__(self, k):
        return _Stub()

    def __contains__(self, k):
        return False

    def __iter__(self):
        return iter(())

    def __len__(self):
        return 0

    def __float__(self):
        return 0.0

    def __int__(self):
        return 0

    def __index__(self):
        return 0

    def __round__(self, n=None):
        return 0

    def __hash__(self):
        return id(self)

    def __eq__(self, other):
        return self is other

    def _num(self, *a):
        return _Stub()

    __or__ = __ror__ = __and__ = __rand__ = __invert__ = __add__ = __radd__ = __sub__ = __rsub__ = _num
    __mul__ = __rmul__ = __truediv__ = __rtruediv__ = __neg__ = __xor__ = _num

    def __lt__(self, o):
        return False

    __gt__ = __le__ = __ge__ = __lt__


class Signal:
    def __init__(self, *types_):
        self.name = None

    def __set_name__(self, owner, name):
        self.name = "_sig_" + name

    def __get__(self, obj, owner=None):
        if obj is None:
            return self
        b = obj.__dict__.get(self.name)
        if b is None:
            b = obj.__dict__[self.name] = _Bound()
        return b


class _Bound:
    def __init__(self):
        self.slots = []

    def connect(self, fn):
        self.slots.append(fn)

    def disconnect(self, fn=None):
        self.slots = [] if fn is None else [s for s in self.slots if s is not fn]

    def emit(self, *a):
        for s in list(self.slots):
            s(*a)


def _cls(name, base=_Stub, **attrs):
    return type(name, (base,), attrs)


class _Module(types.ModuleType):
    def __getattr__(self, name):
        if name.startswith("__"):
            raise AttributeError(name)
        c = _cls(name)
        setattr(self, name, c)
        return c


# ---------------------------------------------------------------- QtCore
QtCore = _Module("PySide6.QtCore")
QtCore.Signal = Signal


class QObject(_Stub):
    def __init__(self, *a, **k):
        pass


class QTimer(QObject):
    timeout = Signal()

    @staticmethod
    def singleShot(_ms, fn, *rest):
        (rest[0] if rest else fn)() if callable(rest[0] if rest else fn) else None


class QPointF:
    def __init__(self, x=0.0, y=0.0):
        self._x, self._y = float(x), float(y)

    def x(self):
        return self._x

    def y(self):
        return self._y

    def __add__(self, o):
        return QPointF(self._x + o.x(), self._y + o.y())

    def __sub__(self, o):
        return QPointF(self._x - o.x(), self._y - o.y())

    def toPoint(self):
        return self


QtCore.QObject = QObject
QtCore.QTimer = QTimer
QtCore.QPointF = QPointF
QtCore.QPoint = QPointF

# ---------------------------------------------------------------- QtGui
QtGui = _Module("PySide6.QtGui")


class QUndoCommand(_Stub):
    def __init__(self, text=""):
        self._text = text

    def id(self):
        return -1

    def mergeWith(self, other):
        return False


class QUndoStack(QObject):
    cleanChanged = Signal(bool)

    def __init__(self, *a):
        self.cmds, self.index, self.clean = [], 0, 0

    def push(self, cmd):
        cmd.redo()
        del self.cmds[self.index:]
        if self.cmds and cmd.id() != -1 and self.cmds[-1].id() == cmd.id() and self.cmds[-1].mergeWith(cmd):
            return
        self.cmds.append(cmd)
        self.index = len(self.cmds)

    def undo(self):
        if self.index:
            self.index -= 1
            self.cmds[self.index].undo()

    def redo(self):
        if self.index < len(self.cmds):
            self.cmds[self.index].redo()
            self.index += 1

    def clear(self):
        self.cmds, self.index, self.clean = [], 0, 0

    def setClean(self):
        self.clean = self.index

    def isClean(self):
        return self.clean == self.index

    def count(self):
        return len(self.cmds)


QtGui.QUndoCommand = QUndoCommand
QtGui.QUndoStack = QUndoStack

# ---------------------------------------------------------------- QtWidgets
QtWidgets = _Module("PySide6.QtWidgets")


class QWidget(QObject):
    def __init__(self, *a, **k):
        pass


QtWidgets.QWidget = QWidget
for _n in ("QMainWindow", "QGraphicsView", "QTabWidget", "QTreeWidget", "QDialog", "QListWidget",
           "QDockWidget", "QSplitter", "QScrollArea", "QLabel",
           "QPushButton", "QTableWidget", "QGraphicsScene", "QToolBar", "QUndoView"):
    setattr(QtWidgets, _n, _cls(_n, QWidget))
QtWidgets.QGraphicsItem = _cls("QGraphicsItem")


class QLineEdit(QWidget):
    editingFinished = Signal()
    textChanged = Signal(str)

    def __init__(self, text="", *a):
        self._t = text if isinstance(text, str) else ""

    def text(self):
        return self._t

    def setText(self, t):
        self._t = t


class QPlainTextEdit(QWidget):
    textChanged = Signal()

    def __init__(self, text="", *a):
        self._t = text if isinstance(text, str) else ""

    def toPlainText(self):
        return self._t

    def setPlainText(self, t):
        self._t = t
        self.textChanged.emit()


class QComboBox(QWidget):
    activated = Signal(int)
    currentIndexChanged = Signal(int)

    def __init__(self, *a):
        self.items, self.cur = [], -1

    def addItem(self, text, data=None):
        self.items.append((text, data))
        if self.cur < 0:
            self.cur = 0

    def addItems(self, texts):
        for t in texts:
            self.addItem(t)

    def insertItem(self, i, text, data=None):
        self.items.insert(i, (text, data))

    def clear(self):
        self.items, self.cur = [], -1

    def count(self):
        return len(self.items)

    def findData(self, d):
        return next((i for i, (_, x) in enumerate(self.items) if x is d or (x == d and x is not None)), -1)

    def findText(self, t):
        return next((i for i, (x, _) in enumerate(self.items) if x == t), -1)

    def itemData(self, i):
        return self.items[i][1] if 0 <= i < len(self.items) else None

    def currentData(self):
        return self.itemData(self.cur)

    def currentText(self):
        return self.items[self.cur][0] if 0 <= self.cur < len(self.items) else ""

    def setCurrentIndex(self, i):
        self.cur = i

    def setCurrentText(self, t):
        i = self.findText(t)
        if i >= 0:
            self.cur = i


QtWidgets.QLineEdit = QLineEdit
QtWidgets.QPlainTextEdit = QPlainTextEdit
QtWidgets.QComboBox = QComboBox


def install():
    pkg = sys.modules[__name__]
    pkg.QtCore, pkg.QtGui, pkg.QtWidgets = QtCore, QtGui, QtWidgets
    sys.modules["PySide6.QtCore"] = QtCore
    sys.modules["PySide6.QtGui"] = QtGui
    sys.modules["PySide6.QtWidgets"] = QtWidgets


install()
