"""Control a Roku TV's volume via the External Control Protocol (ECP).

ECP is a simple HTTP API every Roku device exposes on port 8060. Roku TVs only
support *relative* volume changes (``VolumeUp`` / ``VolumeDown`` / ``VolumeMute``
keypresses) — there is no "set volume to N" command — so this client tracks its
own offset relative to whatever volume the TV was at when we started.

Find the TV's IP with :meth:`RokuTV.discover` (SSDP) or set ``ROKU_IP``.
A pluggable ``transport`` lets tests and ``--roku-dry-run`` capture keypresses
instead of hitting the network.
"""

from __future__ import annotations

import os
import socket
import time
import urllib.request
from typing import Callable

DEFAULT_PORT = 8060


class RokuError(RuntimeError):
    pass


class RokuTV:
    def __init__(
        self,
        ip: str | None = None,
        port: int = DEFAULT_PORT,
        transport: Callable[[str], None] | None = None,
    ):
        self.ip = ip or os.getenv("ROKU_IP")
        self.port = port
        # transport(key) actually delivers a keypress; default hits the device.
        self._transport = transport or self._http_keypress

    # -- low level -----------------------------------------------------------

    def _http_keypress(self, key: str) -> None:
        if not self.ip:
            raise RokuError("no Roku IP set (use --roku-ip, ROKU_IP, or discovery)")
        url = f"http://{self.ip}:{self.port}/keypress/{key}"
        req = urllib.request.Request(url, data=b"", method="POST")
        try:
            urllib.request.urlopen(req, timeout=5)
        except Exception as e:  # noqa: BLE001 - surface any network/HTTP issue
            raise RokuError(f"Roku keypress {key!r} failed: {e}") from e

    def keypress(self, key: str) -> None:
        self._transport(key)

    # -- volume --------------------------------------------------------------

    def volume_up(self, steps: int = 1, pause: float = 0.05) -> None:
        for _ in range(max(0, steps)):
            self.keypress("VolumeUp")
            if pause:
                time.sleep(pause)

    def volume_down(self, steps: int = 1, pause: float = 0.05) -> None:
        for _ in range(max(0, steps)):
            self.keypress("VolumeDown")
            if pause:
                time.sleep(pause)

    def apply_delta(self, delta: int, pause: float = 0.05) -> None:
        """Positive raises volume, negative lowers it, by ``abs(delta)`` steps."""
        if delta > 0:
            self.volume_up(delta, pause)
        elif delta < 0:
            self.volume_down(-delta, pause)

    # -- discovery / info ----------------------------------------------------

    @staticmethod
    def discover(timeout: float = 3.0) -> str | None:
        """Find a Roku on the LAN via SSDP. Returns its IP, or None."""
        msg = "\r\n".join(
            [
                "M-SEARCH * HTTP/1.1",
                "HOST: 239.255.255.250:1900",
                'MAN: "ssdp:discover"',
                "ST: roku:ecp",
                "MX: 3",
                "",
                "",
            ]
        ).encode()
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM, socket.IPPROTO_UDP)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.settimeout(timeout)
        try:
            sock.sendto(msg, ("239.255.255.250", 1900))
            deadline = time.time() + timeout
            while time.time() < deadline:
                try:
                    data, _ = sock.recvfrom(2048)
                except socket.timeout:
                    break
                for line in data.decode(errors="ignore").splitlines():
                    if line.lower().startswith("location:"):
                        loc = line.split(":", 1)[1].strip()
                        # e.g. "http://192.168.1.23:8060/"
                        return loc.split("//", 1)[1].split(":", 1)[0]
        finally:
            sock.close()
        return None

    def is_reachable(self) -> bool:
        if not self.ip:
            return False
        try:
            urllib.request.urlopen(
                f"http://{self.ip}:{self.port}/query/device-info", timeout=4
            )
            return True
        except Exception:  # noqa: BLE001
            return False
