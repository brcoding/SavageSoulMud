from pathlib import Path
import secrets
root=Path('/home/barry/.local/state/savsoul-intermud');root.mkdir(parents=True,exist_ok=True);root.chmod(0o700)
keypath=root/'api.key'
if not keypath.exists():keypath.write_text(secrets.token_urlsafe(48)+'\n')
keypath.chmod(0o600)
env=root/'gateway.env'
if not env.exists():env.write_text('SAVSOUL_I3_API_KEY='+keypath.read_text().strip()+'\nSAVSOUL_I3_GATEWAY_SECRET='+secrets.token_urlsafe(48)+'\n')
env.chmod(0o600)
print('Private Intermud credentials initialized; no credentials printed.')
