#!/usr/bin/env python3
"""Install pinned official CPU inference + model into the WSL user's home."""
from pathlib import Path
import hashlib, subprocess, tarfile, urllib.request

base = Path.home()/'.local/share/savsoul-trainer'
base.mkdir(parents=True, exist_ok=True)
assets = [
 ('runtime.tar.gz', 'https://github.com/ggml-org/llama.cpp/releases/download/b11723/llama-b11723-bin-ubuntu-x64.tar.gz', '8f5652caad0ee6a6473e7eb03a08674f429375554852b83fd7156d550669d069'),
 ('Qwen3-0.6B-Q8_0.gguf', 'https://huggingface.co/Qwen/Qwen3-0.6B-GGUF/resolve/23749fefcc72300e3a2ad315e1317431b06b590a/Qwen3-0.6B-Q8_0.gguf', '9465e63a22add5354d9bb4b99e90117043c7124007664907259bd16d043bb031'),
]
for filename, url, digest in assets:
 target=base/filename
 def checksum(path):
  with path.open('rb') as source:
   return hashlib.file_digest(source, 'sha256').hexdigest()
 if not target.exists() or checksum(target)!=digest:
  print('Downloading '+filename, flush=True)
  temporary=target.with_suffix('.download')
  with urllib.request.urlopen(url,timeout=60) as response, temporary.open('wb') as output:
   while chunk:=response.read(1024*1024):output.write(chunk)
  if checksum(temporary)!=digest:raise RuntimeError('Checksum mismatch: '+filename)
  temporary.replace(target)
 print('Verified '+filename, flush=True)
runtime=base/'runtime';runtime.mkdir(exist_ok=True)
subprocess.run(['systemctl','--user','stop','savsoul-trainer'],check=False)
with tarfile.open(base/'runtime.tar.gz') as archive:archive.extractall(runtime,filter='data')
binary=next(runtime.rglob('llama-server'))
binary.chmod(0o755)
units=Path.home()/'.config/systemd/user';units.mkdir(parents=True,exist_ok=True)
(units/'savsoul-trainer.service').write_text(f'''[Unit]
Description=Savage Soul local trainer question translator
After=network.target

[Service]
ExecStart={binary} --model {base/'Qwen3-0.6B-Q8_0.gguf'} --host 127.0.0.1 --port 4091 --ctx-size 2048 --threads 4 --parallel 1 --jinja --reasoning-budget 0 --no-webui --log-disable
Restart=on-failure
RestartSec=5
Nice=10
MemoryMax=2G

[Install]
WantedBy=default.target
''')
subprocess.run(['systemctl','--user','daemon-reload'],check=True)
subprocess.run(['systemctl','--user','enable','--now','savsoul-trainer'],check=True)
print('Local translator service enabled at 127.0.0.1:4091.',flush=True)
