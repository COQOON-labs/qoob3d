"""Shared fixtures for the werft M1 tests (spec: docs/m1_spec.md).

The tests are a black-box oracle: they create example projects in tmp_path,
talk to the service via its HTTP API or the CLI, and compare against values
derived by hand (box volumes etc.).
"""

from __future__ import annotations

import json
import subprocess
import sys
import textwrap
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Callable

import psutil
import pytest

# CLI entry: the spec defines the console script `werft = "werft.cli:main"`.
# Calling main() through the interpreter of the test run avoids depending on
# the script being on PATH.
CLI_PRELUDE = "import sys; from werft.cli import main; sys.exit(main())"

# Upper bound for any single CLI call. A detached service must not keep the
# pipes open (otherwise subprocess.run would hang until the timeout).
CLI_TIMEOUT_S = 90

# Time the pool may need to become warm (spawn + build123d import, M0: 1.7 s
# per process). Generous because CI machines are slow; it is not an assertion
# of the spec, only a wait limit.
BEREIT_TIMEOUT_S = 60


# --------------------------------------------------------------------------
# Project creation
# --------------------------------------------------------------------------

STANDARD_TOML = """\
[projekt]
name = "testprojekt"
einheit = "mm"

[grenzen]
zeit_s = 30
speicher_mb = 4096

[arbeiter]
anzahl = 2
"""


def toml_mit(zeit_s: float = 30, speicher_mb: int = 4096, anzahl: int = 2) -> str:
    return textwrap.dedent(
        f"""\
        [projekt]
        name = "testprojekt"
        einheit = "mm"

        [grenzen]
        zeit_s = {zeit_s}
        speicher_mb = {speicher_mb}

        [arbeiter]
        anzahl = {anzahl}
        """
    )


def eintrag(id_: str, wert: Any, quelle: str = "Testmessung", status: str = "gemessen") -> dict:
    return {"id": id_, "wert": wert, "quelle": quelle, "status": status}


def lege_projekt_an(
    wurzel: Path,
    *,
    toml: str | None = STANDARD_TOML,
    soll: dict[str, Any] | None = None,
    lib: dict[str, str] | None = None,
    parts: dict[str, str] | None = None,
) -> Path:
    """Create a project folder as described in spec section "Projektordner".

    soll: file name -> list of entries (dumped as JSON) or raw string.
    lib / parts: file name -> Python source (dedented).
    """
    wurzel.mkdir(parents=True, exist_ok=True)
    if toml is not None:
        (wurzel / "werft.toml").write_text(toml, encoding="utf-8")
    (wurzel / "soll").mkdir(exist_ok=True)
    (wurzel / "lib").mkdir(exist_ok=True)
    (wurzel / "parts").mkdir(exist_ok=True)
    for name, inhalt in (soll or {}).items():
        text = inhalt if isinstance(inhalt, str) else json.dumps(inhalt, indent=1)
        (wurzel / "soll" / name).write_text(text, encoding="utf-8")
    for name, quelltext in (lib or {}).items():
        (wurzel / "lib" / name).write_text(textwrap.dedent(quelltext), encoding="utf-8")
    for name, quelltext in (parts or {}).items():
        (wurzel / "parts" / name).write_text(textwrap.dedent(quelltext), encoding="utf-8")
    return wurzel


def schreibe(pfad: Path, quelltext: str) -> None:
    pfad.write_text(textwrap.dedent(quelltext), encoding="utf-8")


@pytest.fixture
def projekt_fabrik(tmp_path: Path) -> Callable[..., Path]:
    """Factory: projekt_fabrik("name", parts={...}, ...) -> project path."""

    def fabrik(name: str = "projekt", **kwargs: Any) -> Path:
        return lege_projekt_an(tmp_path / name, **kwargs)

    return fabrik


# Example project: Kiste reads an object soll value, Deckel depends on Kiste.
# Kiste = Box(40, 30, 20) -> 24000 mm3; Deckel = Box(bbox.X, bbox.Y, 2) of
# Kiste -> 40 * 30 * 2 = 2400 mm3.
BEISPIEL_SOLL = {
    "kiste.json": [
        eintrag("kiste.masse", {"dx": 40, "dy": 30, "dz": 20}, "Zeichnung Blatt 1", "gemessen"),
        eintrag("kiste.farbe_nr", 7, "Vermutung", "annahme"),
    ]
}
BEISPIEL_PARTS = {
    "kiste.py": """\
        from build123d import Box
        from werft import teil, soll

        @teil("Kiste", material="PLA", farbe="weiss")
        def bauen(dep):
            m = soll("kiste.masse")
            return Box(m.dx, m.dy, m.dz)
        """,
    "deckel.py": """\
        from build123d import Box
        from werft import teil

        @teil("Deckel", haengt_von=["Kiste"], material="PLA", farbe="weiss")
        def bauen(dep):
            kiste = dep["Kiste"]
            g = kiste.bounding_box().size
            return Box(g.X, g.Y, 2)
        """,
}


@pytest.fixture
def beispiel_projekt(projekt_fabrik: Callable[..., Path]) -> Path:
    return projekt_fabrik("beispiel", soll=BEISPIEL_SOLL, parts=BEISPIEL_PARTS)


# --------------------------------------------------------------------------
# CLI
# --------------------------------------------------------------------------


class CliErgebnis:
    def __init__(self, proc: Any):
        self.code: int = proc.returncode
        self.stdout: str = proc.stdout
        self.stderr: str = proc.stderr

    def json(self) -> dict:
        try:
            return json.loads(self.stdout)
        except json.JSONDecodeError as e:  # pragma: no cover - diagnostic
            raise AssertionError(
                f"stdout is not valid JSON ({e}).\nstdout:\n{self.stdout}\nstderr:\n{self.stderr}"
            ) from e

    def __repr__(self) -> str:
        return f"CliErgebnis(code={self.code}, stdout={self.stdout!r}, stderr={self.stderr!r})"


def werft_cli(args: list[str], cwd: Path | None = None, timeout: float = CLI_TIMEOUT_S) -> CliErgebnis:
    proc = subprocess.run(
        [sys.executable, "-c", CLI_PRELUDE, *args],
        cwd=str(cwd) if cwd else None,
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    return CliErgebnis(proc)


def dienst_json(projekt: Path) -> dict:
    return json.loads((projekt / ".werft" / "dienst.json").read_text(encoding="utf-8"))


def _dienst_pid(projekt: Path) -> int | None:
    try:
        return int(dienst_json(projekt)["pid"])
    except (OSError, json.JSONDecodeError, KeyError, TypeError, ValueError):
        return None


def _toete_baum(pid: int | None) -> None:
    if pid is None:
        return
    try:
        proc = psutil.Process(pid)
        kinder = proc.children(recursive=True)
    except psutil.NoSuchProcess:
        return
    for p in [*kinder, proc]:
        try:
            p.kill()
        except psutil.NoSuchProcess:
            pass
    psutil.wait_procs([*kinder, proc], timeout=5)


def raeume_dienst_auf(projekt: Path) -> None:
    """Stop the service of a project synchronously; kill leftovers."""
    pid = _dienst_pid(projekt)
    try:
        werft_cli(["-C", str(projekt), "dienst", "stop"], timeout=30)
    except Exception:
        pass
    _toete_baum(pid)


# Asynchronous teardown: the spec limits the whole suite to < 120 s, and a
# graceful stop takes about 2 s. Test teardowns therefore only launch
# `werft dienst stop`; the session finalizer waits for all of them and kills
# whatever is still alive, so no process leaks.
_AUSSTEHENDE_STOPS: list[tuple[subprocess.Popen, int | None]] = []


def stoppe_dienst_spaeter(projekt: Path) -> None:
    pid = _dienst_pid(projekt)
    proc = subprocess.Popen(
        [sys.executable, "-c", CLI_PRELUDE, "-C", str(projekt), "dienst", "stop"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    _AUSSTEHENDE_STOPS.append((proc, pid))


@pytest.fixture(scope="session", autouse=True)
def _alle_dienste_beenden() -> Any:
    yield
    for proc, pid in _AUSSTEHENDE_STOPS:
        try:
            proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            proc.kill()
        _toete_baum(pid)


@pytest.fixture
def cli_projekte() -> Any:
    """Collect projects whose service the CLI may have auto-started; stop them afterwards."""
    projekte: list[Path] = []
    yield projekte
    for p in projekte:
        stoppe_dienst_spaeter(p)


# --------------------------------------------------------------------------
# Service over HTTP
# --------------------------------------------------------------------------

LAUFENDE_ZUSTAENDE = ("wartet", "laeuft")


class Dienst:
    """Thin HTTP client for werftd (spec section "Dienst werftd")."""

    def __init__(self, projekt: Path):
        self.projekt = projekt
        daten = dienst_json(projekt)
        self.port: int = int(daten["port"])
        self.pid: int = int(daten["pid"])

    # -- raw requests -----------------------------------------------------
    def anfrage(self, methode: str, pfad: str, body: Any = None, timeout: float = 120) -> tuple[dict, float]:
        """Return (envelope, wall time in s). Non-2xx answers are parsed too."""
        daten = None if body is None else json.dumps(body).encode("utf-8")
        req = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{pfad}",
            data=daten,
            method=methode,
            headers={"Content-Type": "application/json"},
        )
        t0 = time.perf_counter()
        try:
            with urllib.request.urlopen(req, timeout=timeout) as antwort:
                roh = antwort.read()
        except urllib.error.HTTPError as e:
            roh = e.read()
        dauer = time.perf_counter() - t0
        try:
            return json.loads(roh), dauer
        except json.JSONDecodeError as e:  # pragma: no cover - diagnostic
            raise AssertionError(f"{methode} {pfad}: answer is not JSON: {roh!r}") from e

    def get(self, pfad: str, timeout: float = 120) -> dict:
        return self.anfrage("GET", pfad, timeout=timeout)[0]

    def post(self, pfad: str, body: Any = None, timeout: float = 120) -> dict:
        return self.anfrage("POST", pfad, body if body is not None else {}, timeout=timeout)[0]

    # -- convenience --------------------------------------------------------
    def status(self) -> dict:
        env = self.get("/status")
        assert env["ok"] is True, env
        return env["ergebnis"]

    def auftrag(self, auftrag_id: Any) -> dict:
        # The spec does not say whether the envelope of a job that ended in
        # "fehler" has ok = false; both are accepted, the job data must be there.
        env = self.get(f"/auftrag/{auftrag_id}")
        erg = env.get("ergebnis")
        assert isinstance(erg, dict) and "zustand" in erg, env
        if not env["ok"]:
            assert erg["zustand"] == "fehler", env
            assert isinstance(env.get("fehler"), dict), env
        return erg

    def warte_auftrag(self, auftrag_id: Any, timeout: float = 120, takt: float = 0.05) -> dict:
        ende = time.monotonic() + timeout
        while True:
            a = self.auftrag(auftrag_id)
            if a["zustand"] not in LAUFENDE_ZUSTAENDE:
                return a
            if time.monotonic() > ende:
                raise AssertionError(f"job {auftrag_id} not finished after {timeout} s: {a}")
            time.sleep(takt)

    def bauen_env(self, teile: list[str] | None = None, warten_s: float = 60.0) -> dict:
        return self.post("/bauen", {"teile": teile, "warten_s": warten_s})

    def bauen(self, teile: list[str] | None = None, warten_s: float = 60.0) -> dict:
        """Build and return the build result {"teile", "gebaut", "aus_cache"}.

        If the job did not finish within warten_s, poll /auftrag/<id>.
        """
        env = self.bauen_env(teile, warten_s)
        erg = env.get("ergebnis")
        if isinstance(erg, dict) and "auftrag" in erg and "teile" not in erg:
            a = self.warte_auftrag(erg["auftrag"])
            erg = a["ergebnis"]
        assert isinstance(erg, dict) and "teile" in erg, f"no build result in {env}"
        return erg

    def arbeiter_pids(self) -> set[int]:
        return {p.pid for p in psutil.Process(self.pid).children(recursive=True)}

    def warte_bereit(self, timeout: float = BEREIT_TIMEOUT_S) -> None:
        """Wait until /status answers and all workers are free (pool warm)."""
        ende = time.monotonic() + timeout
        letzte: Any = None
        while time.monotonic() < ende:
            try:
                env = self.get("/status", timeout=5)
                letzte = env
                arb = env["ergebnis"]["arbeiter"]
                # ok may be false when the project itself has a load error
                # (broken soll file); the service is up nevertheless.
                if arb["frei"] == arb["anzahl"] and arb["anzahl"] > 0:
                    return
            except (urllib.error.URLError, ConnectionError, OSError, KeyError, TypeError):
                pass
            time.sleep(0.1)
        raise AssertionError(f"service not ready after {timeout} s, last status: {letzte}")


def starte_dienst(projekt: Path) -> Dienst:
    erg = werft_cli(["-C", str(projekt), "dienst", "start"])
    assert erg.code == 0, f"'werft dienst start' failed: {erg}"
    pfad = projekt / ".werft" / "dienst.json"
    ende = time.monotonic() + BEREIT_TIMEOUT_S
    while not pfad.exists():
        assert time.monotonic() < ende, ".werft/dienst.json was not written"
        time.sleep(0.05)
    # The file may be visible before it is completely written.
    while True:
        try:
            dienst = Dienst(projekt)
            break
        except (json.JSONDecodeError, KeyError):
            assert time.monotonic() < ende, ".werft/dienst.json is not valid"
            time.sleep(0.05)
    dienst.warte_bereit()
    return dienst


@pytest.fixture
def dienst_starter() -> Any:
    """Factory fixture: dienst_starter(projekt) -> Dienst; stopped on teardown."""
    projekte: list[Path] = []

    def start(projekt: Path) -> Dienst:
        projekte.append(projekt)
        return starte_dienst(projekt)

    yield start
    for p in projekte:
        stoppe_dienst_spaeter(p)


# --------------------------------------------------------------------------
# Assertion helpers
# --------------------------------------------------------------------------


def nahe(ist: float, soll: float, rel: float = 1e-6) -> bool:
    """Relative comparison. 1e-6 is the default tolerance because OCCT computes
    volumes of planar/cylindrical solids exactly up to floating rounding
    (M0 measured 7e-13 % deviation); 1e-6 leaves room without hiding errors."""
    return abs(ist - soll) <= rel * max(abs(soll), 1e-12)


def fehler_von(build_ergebnis: dict, teil: str) -> dict:
    eintrag_ = build_ergebnis["teile"][teil]
    assert eintrag_["zustand"] == "fehler", eintrag_
    assert isinstance(eintrag_.get("fehler"), dict), eintrag_
    return eintrag_["fehler"]


def geteilter_dienst(tmp_path_factory: Any, name: str, **projekt_kwargs: Any) -> Any:
    """Generator for module-scoped fixtures: one service shared by several
    read-mostly tests (keeps the suite under the 120 s of the spec).
    Tests using it must use part names no other test of the module builds
    when they assert on "gebaut"."""
    projekt = lege_projekt_an(tmp_path_factory.mktemp("geteilt") / name, **projekt_kwargs)
    try:
        yield starte_dienst(projekt)
    finally:
        stoppe_dienst_spaeter(projekt)
