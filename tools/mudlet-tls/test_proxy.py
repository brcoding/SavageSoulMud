import asyncio
import shutil
import ssl
import subprocess
import tempfile
import unittest
from pathlib import Path
from proxy import Proxy, proxy_header


class ProxyTests(unittest.IsolatedAsyncioTestCase):
    def test_header_uses_socket_addresses(self):
        self.assertEqual(proxy_header(("192.0.2.1", 12345), ("127.0.0.1", 4001)),
                         b"PROXY TCP4 192.0.2.1 127.0.0.1 12345 4001\r\n")
        self.assertEqual(proxy_header(("::1", 12345), ("::1", 4001)),
                         b"PROXY TCP6 ::1 ::1 12345 4001\r\n")
        with self.assertRaises(ValueError):
            proxy_header(("invalid\r\n", 12345), ("127.0.0.1", 4001))

    async def test_relay_includes_header_and_preserves_bytes(self):
        received = asyncio.Future()

        async def backend(reader, writer):
            header = await reader.readline()
            data = await reader.readexactly(4)
            received.set_result((header, data))
            writer.write(b"\xff\xfb\xc9hello\r\n")
            await writer.drain()
            writer.close()

        back = await asyncio.start_server(backend, "127.0.0.1", 0)
        proxy = Proxy(back.sockets[0].getsockname()[1])
        front = await asyncio.start_server(proxy.handle, "127.0.0.1", 0)
        try:
            reader, writer = await asyncio.open_connection("127.0.0.1", front.sockets[0].getsockname()[1])
            writer.write(b"look")
            await writer.drain()
            self.assertEqual(await asyncio.wait_for(reader.read(), 3), b"\xff\xfb\xc9hello\r\n")
            header, data = await received
            self.assertTrue(header.startswith(b"PROXY TCP4 127.0.0.1 127.0.0.1 "))
            self.assertEqual(data, b"look")
            writer.close()
            await writer.wait_closed()
        finally:
            front.close()
            back.close()
            await front.wait_closed()
            await back.wait_closed()
            if proxy.clients:
                await asyncio.wait_for(asyncio.gather(*proxy.clients), 3)

    @unittest.skipUnless(shutil.which("openssl"), "OpenSSL needed for TLS certificate smoke test")
    async def test_tls_verification_and_telnet_relay(self):
        with tempfile.TemporaryDirectory() as directory:
            cert = Path(directory) / "localhost.crt"
            key = Path(directory) / "localhost.key"
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                            "-keyout", str(key), "-out", str(cert), "-subj", "/CN=localhost",
                            "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1"],
                           check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.minimum_version = ssl.TLSVersion.TLSv1_2
            context.load_cert_chain(cert, key)
            received = asyncio.Future()

            async def backend(reader, writer):
                header = await reader.readline()
                received.set_result(header)
                writer.write(b"\xff\xfb\xc9Welcome to Savage Soul.\r\n")
                await writer.drain()
                await reader.read()
                writer.close()

            back = await asyncio.start_server(backend, "127.0.0.1", 0)
            proxy = Proxy(back.sockets[0].getsockname()[1])
            front = await asyncio.start_server(proxy.handle, "127.0.0.1", 0, ssl=context)
            try:
                client_context = ssl.create_default_context(cafile=str(cert))
                reader, writer = await asyncio.open_connection("127.0.0.1", front.sockets[0].getsockname()[1],
                                                               ssl=client_context, server_hostname="localhost")
                self.assertEqual(await asyncio.wait_for(reader.readline(), 3), b"\xff\xfb\xc9Welcome to Savage Soul.\r\n")
                self.assertTrue((await received).startswith(b"PROXY TCP4 127.0.0.1 127.0.0.1 "))
                self.assertIsNotNone(writer.get_extra_info("ssl_object"))
                writer.close()
                await writer.wait_closed()
                with self.assertRaises(ssl.SSLCertVerificationError):
                    await asyncio.open_connection("127.0.0.1", front.sockets[0].getsockname()[1],
                                                  ssl=client_context, server_hostname="wrong.example")
            finally:
                front.close()
                back.close()
                await front.wait_closed()
                await back.wait_closed()
                if proxy.clients:
                    await asyncio.wait_for(asyncio.gather(*proxy.clients), 3)


if __name__ == "__main__":
    unittest.main()
