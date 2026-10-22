"""Offline integration checks: real Telnet parsing, masking controls and origin guards."""
import asyncio
import contextlib
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

from aiohttp import ClientSession
from aiohttp.test_utils import TestServer

spec=importlib.util.spec_from_file_location('mud_web_bridge',Path(__file__).with_name('bridge.py'))
bridge=importlib.util.module_from_spec(spec);spec.loader.exec_module(bridge)


class BrowserBridgeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.proxy=None;self.password=None;self.connections=0;self.peers=[]
        self.allow_password_prompt=asyncio.Event()
        async def fixture(reader,writer):
            self.peers.append(writer)
            try:
                self.connections+=1;self.proxy=await reader.readuntil(b'\r\n')
                writer.write(b'Fixture MUD\r\nCharacter name: ');await writer.drain()
                await reader.readuntil(b'rawr\r\n')
                # Fragment both negotiation and prompt across packets.
                writer.write(b'\xff\xfb');await writer.drain();await asyncio.sleep(.01)
                writer.write(b'\x01');await writer.drain()
                await asyncio.wait_for(self.allow_password_prompt.wait(),5)
                writer.write(b'Password: ');await writer.drain()
                self.password=await reader.readuntil(b'fixture-secret\r\n')
                writer.write(b'\xff\xfc\x01Welcome back.\r\nGoodbye.\r\n');await writer.drain()
            except (asyncio.IncompleteReadError,ConnectionError,asyncio.TimeoutError):pass
            finally:writer.close()
        self.tcp=await asyncio.start_server(fixture,'127.0.0.1',0)
        original=bridge.telnetlib3.open_connection
        async def connect(host,port,**kwargs):
            self.assertEqual((host,port),('127.0.0.1',4003))
            return await original('127.0.0.1',self.tcp.sockets[0].getsockname()[1],**kwargs)
        self.patch=patch.object(bridge.telnetlib3,'open_connection',connect);self.patch.start()
        self.server=TestServer(bridge.create_app());await self.server.start_server()
        self.client=ClientSession()
        self.headers={'Host':'savsoul.com','Origin':'https://savsoul.com','X-Forwarded-For':'203.0.113.99, 198.51.100.23'}

    async def asyncTearDown(self):
        await self.client.close();await self.server.close()
        self.tcp.close();await self.tcp.wait_closed()
        for peer in self.peers:peer.close()
        self.patch.stop()

    async def test_telnet_password_controls_and_final_output(self):
        ws=await self.client.ws_connect(self.server.make_url('/mud-websocket'),headers=self.headers)
        transcript='';secret=False;sent_name=False;sent_password=False
        for _ in range(40):
            message=await ws.receive(timeout=5)
            if message.type.name in ['CLOSE','CLOSED']:break
            packet=json.loads(message.data)
            if packet['type']=='mode':
                secret=packet['secret']
                if secret:self.allow_password_prompt.set()
            if packet['type']=='output':
                self.assertEqual(packet['secret'],secret)
                transcript+=packet['text'];await ws.send_json({'type':'ack','seq':packet['seq']})
                if 'Character name:' in transcript and not sent_name:
                    sent_name=True;await ws.send_json({'type':'line','text':'rawr'})
                if 'Password:' in transcript and not sent_password:
                    self.assertTrue(secret);sent_password=True
                    await ws.send_json({'type':'line','text':'fixture-secret'})
        self.assertIn('Goodbye.',transcript)
        self.assertNotIn('fixture-secret',transcript)
        self.assertEqual(self.proxy,b'PROXY TCP4 198.51.100.23 173.255.210.234 1 443\r\n')
        self.assertTrue(self.password.endswith(b'fixture-secret\r\n'))

    async def test_untrusted_origin_and_arbitrary_target_rejected(self):
        for path,headers in [('/mud-websocket',{**self.headers,'Origin':'https://other.example'}),('/mud-websocket?host=example.com',self.headers),('/mud-websocket',{**self.headers,'X-Forwarded-For':'not-an-ip'})]:
            async with self.client.get(self.server.make_url(path),headers=headers) as response:
                self.assertEqual(response.status,403)
                self.assertEqual(response.headers.get('Connection'),'close')
        async with self.client.get(self.server.make_url('/unknown'),headers=self.headers) as response:
            self.assertEqual(response.status,404);self.assertEqual(response.headers.get('Connection'),'close')
        self.assertEqual(self.connections,0)

    async def test_rejected_upgrade_closes_before_pipelined_forged_request(self):
        reader,writer=await asyncio.open_connection('127.0.0.1',self.server.port)
        def request(origin):
            return ('GET /mud-websocket HTTP/1.1\r\nHost: savsoul.com\r\n'
                'Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\n'
                'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n'
                'X-Forwarded-For: 203.0.113.45\r\nOrigin: '+origin+'\r\n\r\n').encode()
        writer.write(request('https://untrusted.example')+request('https://savsoul.com'));await writer.drain()
        response=await asyncio.wait_for(reader.read(),3)
        self.assertIn(b'403 Forbidden',response);self.assertNotIn(b'101 Switching',response)
        self.assertEqual(self.connections,0)
        writer.close();await writer.wait_closed()


if __name__=='__main__':unittest.main()
