"""Offline relay contract checks: local UDP peer only, no public messages."""
import json, os, pathlib, socket, subprocess, sys, tempfile, threading, unittest

class RelayTests(unittest.TestCase):
    def test_authenticated_fixed_peer_roundtrip(self):
        with tempfile.TemporaryDirectory() as tmp:
            root=pathlib.Path(tmp);key='ab'*32;(root/'key').write_text(key)
            peer=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);peer.bind(('127.0.0.1',0));peer.settimeout(3)
            reserve=socket.socket();reserve.bind(('127.0.0.1',0));port=reserve.getsockname()[1];reserve.close()
            reserve_udp=socket.socket(socket.AF_INET,socket.SOCK_DGRAM);reserve_udp.bind(('127.0.0.1',0));udp_port=reserve_udp.getsockname()[1];reserve_udp.close()
            config={'peers':[{'name':'Fixture','host':'127.0.0.1','port':peer.getsockname()[1]}]};(root/'config').write_text(json.dumps(config))
            process=subprocess.Popen([sys.executable,str(pathlib.Path(__file__).with_name('windows-relay.py')),'--bind','127.0.0.1','--port',str(port),'--udp-bind','127.0.0.1','--udp-port',str(udp_port),'--key-file',str(root/'key'),'--config',str(root/'config')],stdout=subprocess.PIPE,stderr=subprocess.PIPE,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
            try:
                self.assertIn(b'ready',process.stdout.readline())
                bad=socket.create_connection(('127.0.0.1',port),timeout=3);bad.sendall(b'{"key":"wrong"}\n');self.assertEqual(bad.recv(100),b'');bad.close()
                client=socket.create_connection(('127.0.0.1',port),timeout=3);client.sendall((json.dumps({'key':key})+'\n').encode());stream=client.makefile('rb');self.assertTrue(json.loads(stream.readline())['ready'])
                raw=b'REQ:ping|DATA:fixture'
                client.sendall((json.dumps({'peer':'Fixture','data':list(raw)})+'\n').encode())
                data,source=peer.recvfrom(4096);self.assertEqual(data,raw);self.assertEqual(source[1],udp_port)
                peer.sendto(b'REQ:reply|DATA:alive',source)
                response=json.loads(stream.readline());self.assertEqual(bytes(response['data']),b'REQ:reply|DATA:alive');self.assertEqual(response['source_ip'],'127.0.0.1')
                client.sendall(b'{"peer":"not-configured","data":[1]}\n');self.assertEqual(stream.readline(),b'')
                stream.close();client.close()
            finally:
                peer.close();process.terminate();process.communicate(timeout=3)

if __name__=='__main__':unittest.main()
