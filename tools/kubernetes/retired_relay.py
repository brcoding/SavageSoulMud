"""Temporary old-VM forwarding during DNS convergence; never runs a second world."""
import asyncio
import ssl

TARGET = '69.164.205.149'
ACTIVE = 0
IDLE_TIMEOUT = 300


async def pipe(reader, writer):
    while chunk := await asyncio.wait_for(reader.read(65536), IDLE_TIMEOUT):
        writer.write(chunk)
        await writer.drain()


async def relay(reader, writer, port, secure=False, websocket=False):
    global ACTIVE
    remote = None
    tasks = []
    if ACTIVE >= 128:
        writer.close()
        return
    ACTIVE += 1
    try:
        context = ssl.create_default_context(cafile='/home/savsoul/runtime/cacert.pem') if secure else None
        upstream, remote = await asyncio.wait_for(asyncio.open_connection(
            TARGET, port, ssl=context, server_hostname='savsoul.com' if secure else None), 10)
        if websocket:
            line = await asyncio.wait_for(reader.readline(), 10)
            fields = line.split(b' ')
            if len(fields) != 3 or fields[0] != b'GET':
                raise ValueError('Expected websocket upgrade')
            query = fields[1].partition(b'?')[2]
            fields[1] = b'/mud-websocket' + (b'?' + query if query else b'')
            remote.write(b' '.join(fields))
            await remote.drain()
        tasks = [asyncio.create_task(pipe(reader, remote)), asyncio.create_task(pipe(upstream, writer))]
        done, _ = await asyncio.wait(tasks, return_when=asyncio.FIRST_COMPLETED)
        for task in done:
            task.result()
    except (OSError, ValueError, asyncio.TimeoutError):
        pass
    finally:
        for task in tasks:
            if not task.done(): task.cancel()
        await asyncio.gather(*tasks, return_exceptions=True)
        for connection in (writer, remote):
            if connection:
                connection.close()
        ACTIVE -= 1


async def main():
    listeners = []
    for host, local, target, secure, websocket in (
        ('0.0.0.0', 4000, 4000, False, False),
        ('0.0.0.0', 4001, 4001, False, False),
        ('127.0.0.1', 4085, 4081, True, False),
        ('127.0.0.1', 4086, 443, True, True),
    ):
        async def accept(reader, writer, port=target, tls=secure, ws=websocket):
            await relay(reader, writer, port, tls, ws)
        listeners.append(await asyncio.start_server(accept, host, local))
    print('Retired endpoints forward to Kubernetes; no local game or I3 gateway is running.', flush=True)
    await asyncio.gather(*(server.serve_forever() for server in listeners))


if __name__ == '__main__':
    asyncio.run(main())
