"""Foreground container launcher; activation is a separate coordinated cutover."""
import argparse
import json
import os
from pathlib import Path
import sys
import time

HOME = Path('/home/savsoul')
STATE = HOME / 'state/kubernetes'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('service', choices=('grim', 'editor', 'web-terminal', 'intermud', 'trainer', 'release'))
    args = parser.parse_args()
    if not (STATE / 'bootstrap-complete').is_file():
        raise RuntimeError('Candidate runtime has not been prepared')
    if args.service == 'editor':
        while (HOME / 'state/runner/release-in-progress').exists():
            time.sleep(.5)
    if args.service == 'intermud' and not (STATE / 'networks-active').exists():
        print('Intermud waits for coordinated cutover; no gateway process is running.', flush=True)
        while not (STATE / 'networks-active').exists():
            time.sleep(2)
    if args.service == 'grim' and not (STATE / 'networks-active').exists():
        for name in ('intermud', 'grapevine', 'zebedee'):
            path = HOME / ('grim/data/' + name + '.json')
            if path.exists() and json.loads(path.read_text()).get('enabled'):
                raise RuntimeError('Candidate external network adapter must remain disabled')
    if args.service == 'trainer' and 'avx2' not in Path('/proc/cpuinfo').read_text():
        raise RuntimeError('Retained trainer binary requires an AVX2-capable node')
    services = json.loads(Path('/etc/savsoul/services.json').read_text())
    service = services[args.service]
    os.chdir(service['cwd'])
    os.environ.update(service.get('env', {}))
    os.environ.update({'HOME': str(HOME), 'PYTHONDONTWRITEBYTECODE': '1',
                      'SSL_CERT_FILE': '/etc/ssl/certs/ca-certificates.crt',
                      'REQUESTS_CA_BUNDLE': '/etc/ssl/certs/ca-certificates.crt'})
    if args.service == 'grim':
        os.environ['GRIM_PROXY_PORT'] = '4003'
    command = service['command']
    os.execv(command[0], command)


if __name__ == '__main__':
    main()
