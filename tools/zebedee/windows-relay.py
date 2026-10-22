#!/usr/bin/env python3
"""Authenticated, fixed-peer Windows UDP gateway for WSL Zebedee. No arbitrary destinations."""
import argparse, hmac, json, selectors, socket, time
MAX_LINE=65536
MAX_DATAGRAM=12000

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bind',required=True,help='Windows WSL virtual gateway IP only')
    parser.add_argument('--port',type=int,default=4084)
    parser.add_argument('--udp-bind',default='0.0.0.0')
    parser.add_argument('--udp-port',type=int,default=4002)
    parser.add_argument('--key-file',required=True)
    parser.add_argument('--config',required=True)
    args=parser.parse_args()
    # Deliberately disallow exposing the key-authenticated control port on all interfaces.
    if args.bind in ('0.0.0.0','::'): parser.error('--bind must be a specific WSL virtual gateway IP')
    key=open(args.key_file,encoding='utf8').read().strip()
    if len(key)!=64 or any(c not in '0123456789abcdef' for c in key): raise ValueError('key must be 32 random bytes represented as64hex characters')
    with open(args.config,encoding='utf-8-sig') as stream: config=json.load(stream)
    peers={p['name'].casefold(): (socket.gethostbyname(p['host']),p['port']) for p in config['peers']}
    if len(peers)>64: raise ValueError('too many peers')
    udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);udp.bind((args.udp_bind,args.udp_port));udp.setblocking(False)
    if hasattr(socket,'SIO_UDP_CONNRESET'):udp.ioctl(socket.SIO_UDP_CONNRESET,False)
    server=socket.socket();server.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);server.bind((args.bind,args.port));server.listen(2);server.setblocking(False)
    poll=selectors.DefaultSelector();poll.register(server,selectors.EVENT_READ,'listener');poll.register(udp,selectors.EVENT_READ,'udp')
    client=None;buf=bytearray();out=bytearray();authenticated=False;deadline=0;window=time.monotonic();count=0
    def close_client():
        nonlocal client,authenticated,buf,out
        if client is not None:
            try:poll.unregister(client)
            except Exception:pass
            client.close()
        client=None;authenticated=False;buf=bytearray();out=bytearray()
    def queue(value):
        nonlocal out
        line=(json.dumps(value,separators=(',',':'))+'\n').encode()
        if len(out)+len(line)>256*1024: close_client();return
        out.extend(line)
    print(f'Zebedee Windows relay ready: TCP{args.bind}:{args.port}, UDP{args.udp_port}, peers{len(peers)}',flush=True)
    while True:
        now=time.monotonic()
        if now-window>=1:window=now;count=0
        if client is not None and not authenticated and now>deadline:close_client()
        for event,_ in poll.select(.02):
            if event.data=='listener':
                conn,_=server.accept();conn.setblocking(False)
                if client is not None: conn.close();continue
                client=conn;deadline=now+3;poll.register(conn,selectors.EVENT_READ,'client')
            elif event.data=='udp':
                try:data,source=udp.recvfrom(MAX_DATAGRAM+1)
                except OSError:continue
                # Inetd may use a separate sending port, but source IP must be pinned.
                if len(data)>MAX_DATAGRAM or source[0] not in {a[0] for a in peers.values()} or not authenticated:continue
                count+=1
                if count>100:continue
                queue({'source_ip':source[0],'source_port':source[1],'data':list(data)})
            elif event.data=='client':
                if client is None or event.fileobj is not client:continue
                try:data=client.recv(8192)
                except BlockingIOError:continue
                except OSError:close_client();continue
                if not data:close_client();continue
                buf.extend(data)
                if len(buf)>MAX_LINE:close_client();continue
                while b'\n' in buf and client is not None:
                    raw,_,tail=buf.partition(b'\n');buf=bytearray(tail)
                    try:
                        request=json.loads(raw)
                        if not isinstance(request,dict):raise ValueError()
                        if not authenticated:
                            supplied=request.get('key','')
                            if not isinstance(supplied,str) or not hmac.compare_digest(supplied,key):close_client();break
                            authenticated=True;queue({'ready':True});continue
                        peer=request.get('peer','').casefold();data=request.get('data')
                        if peer not in peers or not isinstance(data,list) or len(data)>MAX_DATAGRAM or any(type(b) is not int or b<0 or b>255 for b in data):raise ValueError()
                        count+=1
                        if count<=100:udp.sendto(bytes(data),peers[peer])
                    except (ValueError,TypeError,KeyError,AttributeError,UnicodeError,OSError,RecursionError):close_client();break
        if client is not None and out:
            try:sent=client.send(out);del out[:sent]
            except BlockingIOError:pass
            except OSError:close_client()

if __name__=='__main__':main()
