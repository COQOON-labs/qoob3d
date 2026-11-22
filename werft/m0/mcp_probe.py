"""build123d-mcp praktisch ausprobieren (M0): echter MCP-Client ueber stdio.

Prueft: Teil bauen, messen, validieren, rendern, Druckanalyse, Export,
STEP-Import, und ob der Dienst waehrend einer Endlosschleife antwortet.
Ergebnis: out/m0_mcp_probe.json, Bilder nach out/.

    uv run python m0/mcp_probe.py
"""

from __future__ import annotations

import asyncio
import json
import sys
import time
from pathlib import Path

from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

HIER = Path(__file__).resolve().parent
OUT = HIER.parent / "out"
SERVER = str(Path(sys.executable).with_name("build123d-mcp"))

KRANSAEULE = (HIER / "teile.py").read_text().split("# ---------------------------------------------------------------------------\n# Treppe")[0]


def text(res) -> str:
    return "\n".join(getattr(c, "text", f"<{type(c).__name__}>") for c in res.content)


async def aufruf(s, werkzeug, protokoll, **args):
    t = time.perf_counter()
    try:
        res = await s.call_tool(werkzeug, args)
        ok = not res.is_error if hasattr(res, "is_error") else True
        antwort = text(res)
    except Exception as e:  # noqa: BLE001
        ok, antwort = False, f"{type(e).__name__}: {e}"
    dt = time.perf_counter() - t
    protokoll.append(dict(werkzeug=werkzeug, s=round(dt, 3), ok=ok, antwort=antwort[:1500]))
    print(f"[{dt:7.2f}s] {werkzeug}: {antwort[:200]!r}")
    return antwort


async def sitzung(exec_timeout: int, protokoll: list):
    p = StdioServerParameters(command=SERVER, args=["--exec-timeout", str(exec_timeout)])
    async with stdio_client(p) as (r, w), ClientSession(r, w) as s:
        await s.initialize()
        await aufruf(s, "version", protokoll)
        # Teil aus Datei-Code: Kransaeule. Der Sandkasten erlaubt `from __future__`
        # und `math`? Das wird hier gleich mit geprueft.
        code = KRANSAEULE + "\nteil = kransaeule()\nshow(teil, 'kransaeule')\n"
        await aufruf(s, "execute", protokoll, code=code)
        await aufruf(s, "measure", protokoll, object_name="kransaeule")
        await aufruf(s, "validate", protokoll, object_name="kransaeule")
        await aufruf(
            s, "render_view", protokoll, direction="iso", save_to=str(OUT / "mcp_kransaeule.png")
        )
        await aufruf(s, "analyze_printability", protokoll, object_name="kransaeule")
        await aufruf(s, "export", protokoll, filename=str(OUT / "mcp_kransaeule.step"), format="step")
        # STEP-Import ueber das Werkzeug
        await aufruf(
            s,
            "import_cad_file",
            protokoll,
            path=str(HIER / "step_dl" / "nist_ctc_02_asme1_rc.stp"),
            name="ctc02",
        )
        await aufruf(s, "measure", protokoll, object_name="ctc02")
        await aufruf(s, "session_state", protokoll)

        # Endlosschleife: antwortet der Dienst waehrenddessen?
        async def endlos():
            await aufruf(s, "execute", protokoll, code="while True:\n    pass\n")

        aufgabe = asyncio.create_task(endlos())
        await asyncio.sleep(2)
        t = time.perf_counter()
        await aufruf(s, "health_check", protokoll)
        protokoll.append(dict(notiz="health_check waehrend Endlosschleife", s=time.perf_counter() - t))
        await aufgabe
        # Ist der Sitzungszustand nach dem Abbruch noch da?
        await aufruf(s, "session_state", protokoll)
        await aufruf(s, "measure", protokoll, object_name="kransaeule")


def main():
    OUT.mkdir(exist_ok=True)
    protokoll: list = []
    t = time.perf_counter()
    asyncio.run(sitzung(exec_timeout=15, protokoll=protokoll))
    protokoll.append(dict(gesamt_s=time.perf_counter() - t))
    (OUT / "m0_mcp_probe.json").write_text(json.dumps(protokoll, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
