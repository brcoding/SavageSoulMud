"""Fixed-destination browser MUD bridge. Apache owns public TLS; no traffic logging."""
import asyncio
from collections import Counter, OrderedDict, deque
import contextlib
import ipaddress
import json
import logging
import time

from aiohttp import web, WSMsgType
import telnetlib3
from telnetlib3.client import TelnetClient
from telnetlib3.telopt import NAWS

ORIGINS={'https://savsoul.com','https://www.savsoul.com'}
HOSTS={'savsoul.com','www.savsoul.com','savsoul.com:443','www.savsoul.com:443'}
MAX_BACKLOG=128*1024


class BrowserTelnet(TelnetClient):
    def __init__(self, *args, client_ip, **kwargs):
        super().__init__(*args,**kwargs)
        self.client_ip=client_ip
        self.mode_changed=asyncio.Event()

    def connection_made(self, transport):
        # The trusted ingress requires this line before any Telnet negotiation.
        v6=ipaddress.ip_address(self.client_ip).version==6
        header=f"PROXY {'TCP6' if v6 else 'TCP4'} {self.client_ip} {'::1' if v6 else '173.255.210.234'} 1 443\r\n"
        transport.write(header.encode('ascii'))
        super().connection_made(transport)
        # telnetlib3 5 parses received data asynchronously. Observe actual option
        # handlers so ECHO-only changes reach the browser before any later text.
        for name in ['handle_will','handle_wont']:
            original=getattr(self.writer,name)
            def option_changed(option,handler=original):
                before=self.writer.will_echo
                result=handler(option)
                if self.writer.will_echo!=before:self.mode_changed.set()
                return result
            setattr(self.writer,name,option_changed)
        self.mode_changed.set()

    def resize(self,cols,rows):
        self._extra.update(cols=cols,rows=rows)
        if self.writer.local_option.enabled(NAWS):self.writer._send_naws()


class Bridge:
    def __init__(self):
        self.active=Counter();self.attempts=OrderedDict();self.sockets=set()

    def admit(self,ip):
        now=time.monotonic()
        attempts=self.attempts.pop(ip,deque(maxlen=12))
        while attempts and now-attempts[0]>60:attempts.popleft()
        self.attempts[ip]=attempts
        while len(self.attempts)>1024:self.attempts.popitem(last=False)
        if len(self.sockets)>=64 or self.active[ip]>=4 or len(attempts)>=10:return False
        attempts.append(now)
        return True

    async def websocket(self,request):
        if (request.remote!='127.0.0.1' or request.host.lower() not in HOSTS
            or request.headers.get('Origin') not in ORIGINS or request.query_string):
            raise web.HTTPForbidden(text='Use the browser terminal on savsoul.com.')
        # Apache mod_proxy appends its actual connection address as the final
        # X-Forwarded-For hop during handshake construction (after header hooks).
        # Never select the client-controlled first hop or a custom input header.
        try:
            hops=','.join(request.headers.getall('X-Forwarded-For')).split(',')
            ip=str(ipaddress.ip_address(hops[-1].strip()))
        except (KeyError,ValueError):raise web.HTTPForbidden(text='Missing trusted proxy address.')
        if not self.admit(ip):raise web.HTTPTooManyRequests(text='Too many connections. Try again shortly.')
        ws=web.WebSocketResponse(heartbeat=30,max_msg_size=4096,compress=False)
        self.active[ip]+=1;self.sockets.add(ws)
        writer=None;tasks=[];protocols=[]
        try:
            await ws.prepare(request)
            def factory(**kwargs):
                protocol=BrowserTelnet(client_ip=ip,**kwargs);protocols.append(protocol);return protocol
            reader,writer=await asyncio.wait_for(telnetlib3.open_connection(
                '127.0.0.1',4003,client_factory=factory,encoding='utf8',force_binary=True,
                term='XTERM-256COLOR',cols=100,rows=30,connect_minwait=0,
                connect_maxwait=.25,connect_timeout=3,compression=False,limit=16384,
                send_environ=['TERM','COLUMNS','LINES','LANG']),5)
            protocol=protocols[0]
            outstanding=OrderedDict();sequence=0;pending_bytes=0;acked=0
            async def send(packet):
                await asyncio.wait_for(ws.send_json(packet),10)

            async def output():
                nonlocal sequence,pending_bytes
                read_task=asyncio.create_task(reader.read(2048))
                mode_task=asyncio.create_task(protocol.mode_changed.wait())
                previous=None
                try:
                    while not ws.closed:
                        done,_=await asyncio.wait([read_task,mode_task],timeout=15,return_when=asyncio.FIRST_COMPLETED)
                        secret=bool(writer.will_echo)
                        if secret!=previous:
                            await send({'type':'mode','secret':secret});previous=secret
                        if mode_task in done:
                            protocol.mode_changed.clear()
                            mode_task=asyncio.create_task(protocol.mode_changed.wait())
                        if outstanding and time.monotonic()-next(iter(outstanding.values()))[1]>30:
                            await ws.close(code=1008,message=b'Terminal is not consuming output.');return
                        if read_task not in done:continue
                        text=read_task.result()
                        if not text:
                            # Let xterm acknowledge the final output before closing.
                            deadline=time.monotonic()+2
                            while outstanding and time.monotonic()<deadline:await asyncio.sleep(.02)
                            await ws.close(code=1000,message=b'MUD connection ended.');return
                        size=len(text.encode('utf8'))
                        if pending_bytes+size>MAX_BACKLOG:
                            await ws.close(code=1008,message=b'Terminal output limit exceeded.');return
                        sequence+=1;outstanding[sequence]=(size,time.monotonic());pending_bytes+=size
                        await send({'type':'output','seq':sequence,'text':text,'secret':secret})
                        read_task=asyncio.create_task(reader.read(2048))
                finally:
                    read_task.cancel();mode_task.cancel()
                    await asyncio.gather(read_task,mode_task,return_exceptions=True)

            async def input_loop():
                nonlocal pending_bytes,acked
                messages=deque();commands=deque();resizes=deque()
                async for message in ws:
                    if message.type!=WSMsgType.TEXT:break
                    now=time.monotonic();messages.append(now)
                    while messages and now-messages[0]>1:messages.popleft()
                    if len(messages)>256:raise ValueError('Message rate exceeded')
                    packet=json.loads(message.data)
                    if not isinstance(packet,dict):raise ValueError('Invalid message')
                    kind=packet.get('type')
                    if kind=='ack':
                        value=packet.get('seq')
                        if type(value)!=int or value<acked or value>sequence:raise ValueError('Invalid output acknowledgement')
                        acked=value
                        while outstanding and next(iter(outstanding))<=value:pending_bytes-=outstanding.popitem(last=False)[1][0]
                    elif kind=='line':
                        text=packet.get('text')
                        if not isinstance(text,str) or len(text)>2048 or any(not 32<=ord(c)<=126 for c in text):raise ValueError('Invalid command line')
                        commands.append(now)
                        while commands and now-commands[0]>10:commands.popleft()
                        if len(commands)>30:raise ValueError('Command rate exceeded')
                        writer.write(text+'\r\n');await asyncio.wait_for(writer.drain(),5)
                    elif kind=='resize':
                        cols,rows=packet.get('cols'),packet.get('rows')
                        if type(cols)!=int or type(rows)!=int or not 20<=cols<=240 or not 8<=rows<=100:raise ValueError('Invalid dimensions')
                        resizes.append(now)
                        while resizes and now-resizes[0]>1:resizes.popleft()
                        if len(resizes)<=10:protocol.resize(cols,rows)
                    else:raise ValueError('Unknown message')
            tasks=[asyncio.create_task(output()),asyncio.create_task(input_loop())]
            done,_=await asyncio.wait(tasks,timeout=24*3600,return_when=asyncio.FIRST_COMPLETED)
            for task in done:task.result()
        except (OSError,ValueError,asyncio.TimeoutError,ConnectionError):
            if ws.prepared and not ws.closed:
                with contextlib.suppress(Exception):await ws.send_json({'type':'notice','text':'The connection ended. You can reconnect.'})
        finally:
            for task in tasks:task.cancel()
            await asyncio.gather(*tasks,return_exceptions=True)
            if writer is None and protocols:writer=getattr(protocols[0],'writer',None)
            if writer is not None:writer.close()
            if ws.prepared:await ws.close()
            self.sockets.discard(ws);self.active[ip]-=1
            if not self.active[ip]:del self.active[ip]
        return ws

    async def shutdown(self,app):
        await asyncio.gather(*(ws.close(code=1001,message=b'Browser bridge restarting.') for ws in tuple(self.sockets)),return_exceptions=True)


def create_app():
    # Apache 2.4.7 tunnels immediately, even for rejected upgrades. A reusable
    # backend HTTP connection would let later requests skip Apache's header
    # sanitization. Every non-upgraded response must close the connection.
    @web.middleware
    async def close_http(request,handler):
        try:response=await handler(request)
        except web.HTTPException as error:
            error.force_close();error.headers['Connection']='close';raise
        if not isinstance(response,web.WebSocketResponse) or not response.prepared or response.status!=101:
            response.force_close();response.headers['Connection']='close'
        return response
    bridge=Bridge();app=web.Application(client_max_size=4096,middlewares=[close_http])
    app.router.add_get('/mud-websocket',bridge.websocket)
    async def health(request):return web.json_response({'status':'ready','connections':len(bridge.sockets)})
    app.router.add_get('/health',health);app.on_shutdown.append(bridge.shutdown)
    return app

if __name__=='__main__':
    logging.basicConfig(level=logging.WARNING)
    logging.getLogger('telnetlib3').setLevel(logging.CRITICAL)
    web.run_app(create_app(),host='127.0.0.1',port=4086,access_log=None,print=None,keepalive_timeout=0)
