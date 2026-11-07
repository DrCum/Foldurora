#!/usr/bin/env python3
"""Rasterize Foldurora's supplied SVG and package its ICO/ICNS variants.

This developer tool uses installed librsvg/cairo shared libraries. The viewer
build consumes committed assets and does not need Python or these libraries.
"""

import argparse
import ctypes
import ctypes.util
import hashlib
import json
from pathlib import Path
import re
import struct
import tempfile
import xml.etree.ElementTree as ElementTree


class Rectangle(ctypes.Structure):
    _fields_ = [(name, ctypes.c_double) for name in ("x", "y", "width", "height")]


class GError(ctypes.Structure):
    _fields_ = [("domain", ctypes.c_uint), ("code", ctypes.c_int), ("message", ctypes.c_char_p)]


def library(name):
    path = ctypes.util.find_library(name)
    if not path:
        raise RuntimeError(f"Install the development renderer's {name} shared library first")
    return ctypes.CDLL(path)


def vector_source(path):
    source = path.read_bytes()
    if len(source) > 2 * 1024 * 1024:
        raise ValueError("Brand artwork must be a bounded self-contained SVG")
    text = source.decode("utf-8")
    if any(marker in text.casefold() for marker in ("<!doctype", "<!entity", "@import")):
        raise ValueError("External SVG declarations are not supported")
    root = ElementTree.fromstring(source)
    if root.tag.rsplit("}", 1)[-1] != "svg":
        raise ValueError("Expected SVG artwork")
    for element in root.iter():
        if element.tag.rsplit("}", 1)[-1].casefold() in ("script", "foreignobject"):
            raise ValueError("Brand artwork must contain vector geometry only")
        values = list(element.attrib.values()) + [element.text or ""]
        for value in values:
            for target in re.findall(r"url\(\s*['\"]?([^)'\"]+)", value, flags=re.IGNORECASE):
                if not target.startswith("#"):
                    raise ValueError("SVG paint references must stay within the artwork")
        for attribute, value in element.attrib.items():
            name = attribute.rsplit("}", 1)[-1].casefold()
            if name.startswith("on"):
                raise ValueError("SVG event handlers are not supported")
            if name in ("href", "src") and not value.startswith("#"):
                raise ValueError("SVG references must stay within the artwork")
    return source


class Renderer:
    def __init__(self, source):
        self.rsvg = library("rsvg-2")
        self.cairo = library("cairo")
        self.gobject = library("gobject-2.0")
        self.glib = library("glib-2.0")
        self.rsvg.rsvg_handle_new_from_data.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.POINTER(ctypes.c_void_p)]
        self.rsvg.rsvg_handle_new_from_data.restype = ctypes.c_void_p
        self.rsvg.rsvg_handle_render_document.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(Rectangle), ctypes.POINTER(ctypes.c_void_p)]
        self.rsvg.rsvg_handle_render_document.restype = ctypes.c_int
        self.cairo.cairo_image_surface_create.argtypes = [ctypes.c_int, ctypes.c_int, ctypes.c_int]
        self.cairo.cairo_image_surface_create.restype = ctypes.c_void_p
        self.cairo.cairo_create.argtypes = [ctypes.c_void_p]
        self.cairo.cairo_create.restype = ctypes.c_void_p
        self.cairo.cairo_surface_status.argtypes = [ctypes.c_void_p]
        self.cairo.cairo_surface_status.restype = ctypes.c_int
        self.cairo.cairo_surface_write_to_png.argtypes = [ctypes.c_void_p, ctypes.c_char_p]
        self.cairo.cairo_surface_write_to_png.restype = ctypes.c_int
        self.cairo.cairo_destroy.argtypes = [ctypes.c_void_p]
        self.cairo.cairo_surface_destroy.argtypes = [ctypes.c_void_p]
        self.cairo.cairo_version_string.restype = ctypes.c_char_p
        self.gobject.g_object_unref.argtypes = [ctypes.c_void_p]
        self.glib.g_error_free.argtypes = [ctypes.c_void_p]
        self.error = ctypes.c_void_p()
        self.buffer = ctypes.create_string_buffer(source)
        self.handle = self.rsvg.rsvg_handle_new_from_data(self.buffer, len(source), ctypes.byref(self.error))
        self.check(self.handle, "SVG parse failed")

    def check(self, success, message):
        if self.error.value:
            error = ctypes.cast(self.error, ctypes.POINTER(GError)).contents
            detail = error.message.decode("utf-8", "replace")
            self.glib.g_error_free(self.error)
            self.error = ctypes.c_void_p()
            raise RuntimeError(f"{message}: {detail}")
        if not success:
            raise RuntimeError(message)

    def render(self, size, path):
        if not 1 <= size <= 2048:
            raise ValueError("Unsupported icon size")
        surface = self.cairo.cairo_image_surface_create(0, size, size)
        self.check(surface and self.cairo.cairo_surface_status(surface) == 0, "Cannot allocate icon surface")
        context = self.cairo.cairo_create(surface)
        try:
            viewport = Rectangle(0, 0, size, size)
            self.check(self.rsvg.rsvg_handle_render_document(self.handle, context, ctypes.byref(viewport), ctypes.byref(self.error)), "SVG rendering failed")
            if self.cairo.cairo_surface_write_to_png(surface, str(path).encode()) != 0:
                raise RuntimeError(f"Cannot write {path.name}")
        finally:
            self.cairo.cairo_destroy(context)
            self.cairo.cairo_surface_destroy(surface)

    def close(self):
        self.gobject.g_object_unref(self.handle)
        self.handle = None

    def version(self):
        parts = [ctypes.c_uint.in_dll(self.rsvg, f"rsvg_{part}_version").value for part in ("major", "minor", "micro")]
        return {"librsvg": ".".join(map(str, parts)), "cairo": self.cairo.cairo_version_string().decode("ascii")}


def ico(frames):
    offset = 6 + 16 * len(frames)
    entries = []
    payload = []
    for size, data in frames:
        entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
        payload.append(data)
        offset += len(data)
    return struct.pack("<HHH", 0, 1, len(frames)) + b"".join(entries + payload)


def icns(frames):
    chunks = [kind.encode("ascii") + struct.pack(">I", 8 + len(data)) + data for kind, data in frames]
    body = b"".join(chunks)
    return b"icns" + struct.pack(">I", 8 + len(body)) + body


def main():
    repository = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=repository / "assets/branding/foldurora-logo.svg")
    parser.add_argument("--out", type=Path, default=repository / "assets/branding")
    args = parser.parse_args()
    source = vector_source(args.source)
    args.out.mkdir(parents=True, exist_ok=True)
    renderer = Renderer(source)
    icon_sizes = [16, 24, 32, 48, 64, 128, 256]
    mac_sizes = [("icp4", 16), ("icp5", 32), ("icp6", 64), ("ic07", 128), ("ic08", 256), ("ic09", 512), ("ic10", 1024), ("ic11", 32), ("ic12", 64), ("ic13", 256), ("ic14", 512)]
    try:
        with tempfile.TemporaryDirectory(prefix="foldurora-branding-") as temporary:
            frames = {}
            for size in sorted(set(icon_sizes + [40, 512, 720, 1024])):
                path = Path(temporary) / f"{size}.png"
                renderer.render(size, path)
                frames[size] = path.read_bytes()
            assets = {
                "foldurora-logo.png": frames[720],
                "foldurora-mark.png": frames[40],
                "foldurora-window.png": frames[64],
                "foldurora.ico": ico([(size, frames[size]) for size in icon_sizes]),
                "foldurora.icns": icns([(kind, frames[size]) for kind, size in mac_sizes]),
            }
            for name, data in assets.items():
                (args.out / name).write_bytes(data)
            manifest = {
                "schema": 1,
                "source": args.source.name,
                "source_sha256": hashlib.sha256(source).hexdigest(),
                "renderer": renderer.version(),
                "png_sizes": {"foldurora-logo.png": 720, "foldurora-mark.png": 40, "foldurora-window.png": 64},
                "ico_sizes": icon_sizes,
                "icns_sizes": dict(mac_sizes),
                "sha256": {name: hashlib.sha256(data).hexdigest() for name, data in assets.items()},
            }
            (args.out / "foldurora-branding.json").write_text(json.dumps(manifest, indent=2) + "\n")
    finally:
        renderer.close()


if __name__ == "__main__":
    main()
