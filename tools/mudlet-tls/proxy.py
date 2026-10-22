#!/usr/bin/env python3
"""Independent TLS ingress: TLS sockets survive the MUD's plain-socket copyover."""
import argparse
import asyncio
import contextlib
import ipaddress
import logging
import signal
import ssl
from pathlib import Path

LOG = logging.getLogger("savsoul-tls")


def proxy_header(peer, destination):
    """Only service-owned socket metadata becomes the trusted PROXYv1 header."""
    source = ipaddress.ip_address(peer[0])
    target = ipaddress.ip_address(destination[0])
    if source.version != target.version:
        # A dual-stack listener can return an IPv4-mapped IPv6 source.
        if source.version == 6 and source.ipv4_mapped and target.version == 4:
            source = source.ipv4_mapped
        elif target.version == 6 and target.ipv4_mapped and source.version == 4:
            target = target.ipv4_mapped
        else:
            raise ValueError("source and destination address families differ")
    return (f"PROXY TCP{source.version} {source} {target} {peer[1]} {destination[1]}\r\n").encode("ascii")


async def pump(reader, writer):
    while data := await reader.read(16384):
        writer.write(data)
        await asyncio.wait_for(writer.drain(), 15)


class Proxy:
    def __init__(self, backend_port, max_connections=128):
        self.backend_port = backend_port
        self.max_connections = max_connections
        self.clients = set()

    async def handle(self, reader, writer):
        task = asyncio.current_task()
        if len(self.clients) >= self.max_connections:
            writer.close()
            return
        self.clients.add(task)
        backend_writer = None
        pumps = []
        try:
            backend_reader, backend_writer = await asyncio.wait_for(
                asyncio.open_connection("127.0.0.1", self.backend_port), 3
            )
            backend_writer.write(proxy_header(writer.get_extra_info("peername"), writer.get_extra_info("sockname")))
            await asyncio.wait_for(backend_writer.drain(), 3)
            pumps = [asyncio.create_task(pump(reader, backend_writer)),
                     asyncio.create_task(pump(backend_reader, writer))]
            await asyncio.wait(pumps, return_when=asyncio.FIRST_COMPLETED)
        except (OSError, ValueError, asyncio.TimeoutError):
            # Do not log raw traffic or credentials, including login failures.
            LOG.debug("TLS connection ended before relay completed")
        finally:
            for relay in pumps:
                relay.cancel()
            await asyncio.gather(*pumps, return_exceptions=True)
            for stream in (backend_writer, writer):
                if stream is not None:
                    stream.close()
                    with contextlib.suppress(OSError, asyncio.TimeoutError):
                        await asyncio.wait_for(stream.wait_closed(), 2)
            self.clients.discard(task)


async def serve(args):
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(args.cert, args.key)
    proxy = Proxy(args.backend_port)
    server = await asyncio.start_server(proxy.handle, args.host, args.port,
                                        ssl=context, ssl_handshake_timeout=10,
                                        limit=32768, backlog=128)
    stopping = asyncio.Event()
    loop = asyncio.get_running_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stopping.set)
    LOG.info("TLS listening on %s:%s, trusted backend 127.0.0.1:%s", args.host, args.port, args.backend_port)
    async with server:
        await stopping.wait()
        server.close()
        clients = list(proxy.clients)
        for task in clients:
            task.cancel()
        await asyncio.gather(*clients, return_exceptions=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=4001)
    parser.add_argument("--backend-port", type=int, default=4003)
    parser.add_argument("--cert", type=Path, required=True)
    parser.add_argument("--key", type=Path, required=True)
    args = parser.parse_args()
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(message)s")
    asyncio.run(serve(args))


if __name__ == "__main__":
    main()
