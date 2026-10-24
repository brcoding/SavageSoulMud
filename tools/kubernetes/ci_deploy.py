"""Upload one checked artifact through the restricted named-pod tunnel."""
import argparse
import base64
import http.client
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import urllib.request


def configuration():
    server = os.environ['SAVSOUL_KUBE_SERVER']
    if not server.startswith('https://'):
        raise ValueError('Kubernetes must use verified HTTPS')
    ca = os.environ['SAVSOUL_KUBE_CA']
    base64.b64decode(ca, validate=True)
    return {'apiVersion': 'v1', 'kind': 'Config', 'current-context': 'release',
            'clusters': [{'name': 'cluster', 'cluster': {'server': server, 'certificate-authority-data': ca}}],
            'users': [{'name': 'release', 'user': {'token': os.environ['SAVSOUL_KUBE_TOKEN']}}],
            'contexts': [{'name': 'release', 'context': {'cluster': 'cluster', 'user': 'release', 'namespace': 'savsoul'}}]}


def wait_tunnel(process, deadline=30):
    # Capture only readiness, never dump kubectl credential/error diagnostics.
    end = time.monotonic() + deadline
    while time.monotonic() < end:
        if process.poll() is not None:
            raise RuntimeError('Restricted Kubernetes tunnel exited before readiness')
        try:
            with urllib.request.urlopen('http://127.0.0.1:49192/healthz', timeout=1) as response:
                if response.status == 200:
                    return
        except OSError:
            time.sleep(.2)
    raise TimeoutError('Private release tunnel did not become ready')


def upload(artifact, revision, port=49192):
    connection = http.client.HTTPConnection('127.0.0.1', port, timeout=300)
    try:
        connection.putrequest('POST', '/release/' + revision)
        connection.putheader('Authorization', 'Bearer ' + os.environ['SAVSOUL_KUBE_RELEASE_TOKEN'])
        connection.putheader('Content-Length', str(artifact.stat().st_size))
        connection.putheader('Content-Type', 'application/gzip')
        connection.endheaders()
        with artifact.open('rb') as stream:
            while chunk := stream.read(256 * 1024):
                connection.send(chunk)
        response = connection.getresponse()
        raw = response.read(16385)
        if len(raw) > 16384 or response.status != 200:
            raise RuntimeError('Kubernetes receiver rejected the release (HTTP {})'.format(response.status))
        result = json.loads(raw)
        if result.get('revision') != revision:
            raise RuntimeError('Receiver did not confirm the requested revision')
        return result
    finally:
        connection.close()


def wait_services(revision, port=49192, deadline=180):
    end = time.monotonic() + deadline
    while time.monotonic() < end:
        try:
            with urllib.request.urlopen('http://127.0.0.1:{}/ready'.format(port), timeout=3) as response:
                if json.load(response).get('revision') == revision:
                    return
        except OSError:
            pass
        time.sleep(1)
    raise RuntimeError('Release committed but game/editor/web readiness did not recover')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--artifact', type=Path, required=True)
    args = parser.parse_args()
    if not re.fullmatch('[0-9a-f]{40}', args.revision):
        raise ValueError('Expected full tested Git revision')
    os.umask(0o077)
    with tempfile.TemporaryDirectory(prefix='savsoul-kube-') as folder:
        path = Path(folder) / 'config.json'
        path.write_text(json.dumps(configuration()))
        path.chmod(0o600)
        env = {**os.environ, 'KUBECONFIG': str(path)}
        # The token belongs only in the private config, not in child environment.
        env.pop('SAVSOUL_KUBE_TOKEN', None)
        env.pop('SAVSOUL_KUBE_RELEASE_TOKEN', None)
        with tempfile.TemporaryFile() as errors:
            process = subprocess.Popen(['kubectl', '-n', 'savsoul', 'port-forward', '--address=127.0.0.1',
                                        'pod/savsoul-0', '49192:4092'], env=env,
                                       stdout=errors, stderr=errors)
            try:
                wait_tunnel(process)
                result = upload(args.artifact, args.revision)
                wait_services(args.revision)
                print('Kubernetes candidate released {} ({})'.format(result['version'], result['revision']))
                summary = os.environ.get('GITHUB_STEP_SUMMARY')
                if summary:
                    with open(summary, 'a') as out:
                        out.write('Released `{}` (`{}`) to Kubernetes candidate `savsoul/savsoul-0` using verified copyover.\n'.format(result['version'], result['revision']))
            finally:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=10)


if __name__ == '__main__':
    main()
