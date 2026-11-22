"""Startet build123d-mcp ueber stdio und listet Werkzeuge mit Beschreibung."""
import asyncio, json, sys, time
from mcp import ClientSession, StdioServerParameters
from mcp.client.stdio import stdio_client

async def main():
    p = StdioServerParameters(command=str(__import__("pathlib").Path(sys.executable).with_name("build123d-mcp")), args=[])
    t0 = time.perf_counter()
    async with stdio_client(p) as (r, w):
        async with ClientSession(r, w) as s:
            await s.initialize()
            t1 = time.perf_counter()
            tools = (await s.list_tools()).tools
            print(f"# start+initialize: {t1-t0:.2f} s, tools: {len(tools)}")
            for t in tools:
                props = list((t.input_schema or {}).get("properties", {}).keys())
                print(f"## {t.name}({', '.join(props)})\n{(t.description or '').strip()[:600]}\n")
asyncio.run(main())
