"""Render secret-free Kubernetes resources from the reviewed deployment inputs."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DEST = ROOT / 'deploy/kubernetes'
PYTHON = 'docker.io/library/python:3.12-slim-trixie@sha256:a0f57064cbdcd6285fa45f51237ab0b35ea8ebaed06a1f5973e89e19dfb86704'
NGINX = 'docker.io/library/nginx:stable-alpine@sha256:47928bf67eb62a1863cb08d9ecb52d3e704304ba614eaee17884204677702951'
LABEL = {'app.kubernetes.io/name': 'savsoul'}
HOME = '/home/savsoul/'


def mount(path, readonly=False, target=None):
    return {'name': 'home', 'mountPath': target or HOME + path,
            'subPath': path, 'readOnly': readonly}


def container(name, mounts, memory, request):
    return {'name': name, 'image': PYTHON,
            'command': ['python', '/etc/savsoul/launch.py', name],
            'securityContext': {'allowPrivilegeEscalation': False,
                                'readOnlyRootFilesystem': True,
                                'capabilities': {'drop': ['ALL']}},
            'resources': {'requests': {'cpu': '25m', 'memory': request},
                          'limits': {'cpu': '750m', 'memory': memory}},
            'volumeMounts': mounts + [
                {'name': 'config', 'mountPath': '/etc/savsoul', 'readOnly': True},
                {'name': name + '-tmp', 'mountPath': '/tmp'}]}


def containers():
    runtime = [mount('runtime/venv', True), mount('runtime/python', True)]
    marker = mount('state/kubernetes', True)
    game = container('grim', [mount('grim'), mount('state/runner'), marker,
                             mount('public', target='/var/www/lpmud')] + runtime, '128Mi', '64Mi')
    game['ports'] = [{'name': 'telnet', 'containerPort': 4000}]
    game['readinessProbe'] = {'exec': {'command': ['python', '-c',
        "from pathlib import Path; p=Path('/home/savsoul/state/runner/mud.pid').read_text().strip(); "
        "assert Path('/proc/'+p+'/exe').resolve()==Path('/home/savsoul/grim/bin/savsoul'); "
        "assert any(len(f)>3 and f[1].endswith(':0FA0') and f[3]=='0A' "
        "for f in (line.split() for line in Path('/proc/net/tcp').read_text().splitlines()[1:]))"]},
        'periodSeconds': 5}
    editor = container('editor', [mount('grim'), mount('state/editor'),
                                 mount('state/runner'), marker,
                                 mount('public', target='/var/www/lpmud')] + runtime,
                       '128Mi', '64Mi')
    web = container('web-terminal', [mount('grim/tools/web-terminal', True), marker]
                    + runtime, '128Mi', '48Mi')
    for app, port in ((editor, 4085), (web, 4086)):
        app['readinessProbe'] = {'exec': {'command': ['python', '-c',
            f"import socket; socket.create_connection(('127.0.0.1',{port}),timeout=2).close()"]},
            'periodSeconds': 5}
    gateway = container('intermud', [mount('runtime/i3-gateway', True),
                        mount('.local/state/savsoul-intermud'),
                        mount('grim/tools/intermud', True), marker] + runtime,
                        '192Mi', '64Mi')
    trainer = container('trainer', [mount('runtime/trainer', True), marker], '896Mi', '768Mi')
    trainer['startupProbe'] = {'exec': {'command': ['python', '-c',
        "import urllib.request; urllib.request.urlopen('http://127.0.0.1:4091/health',timeout=2)"]},
        'periodSeconds': 5, 'failureThreshold': 60}
    nginx = container('nginx', [mount('public', True, '/var/www/lpmud')], '64Mi', '32Mi')
    nginx['volumeMounts'].append({'name': 'tls', 'mountPath': '/run/savsoul-tls', 'readOnly': True})
    # A different UID prevents same-pod services reading the key via /proc/PID/root.
    nginx['securityContext'].update({'runAsUser': 1012, 'runAsGroup': 1012})
    nginx['image'] = NGINX
    nginx['command'] = ['nginx', '-c', '/etc/savsoul/nginx.conf', '-g', 'daemon off;']
    nginx['readinessProbe'] = {'httpGet': {'path': '/healthz', 'port': 9080}, 'periodSeconds': 5}
    release = container('release', [mount('grim'), mount('state/releases'), mount('state/runner'), marker,
                                   mount('public', target='/var/www/lpmud')], '128Mi', '48Mi')
    release['volumeMounts'].append({'name': 'release-auth', 'mountPath': '/run/savsoul-release', 'readOnly': True})
    release['readinessProbe'] = {'exec': {'command': ['python', '-c',
        "from pathlib import Path; assert any(len(f)>3 and f[1].endswith(':0FFC') and f[3]=='0A' "
        "for f in (line.split() for line in Path('/proc/net/tcp').read_text().splitlines()[1:]))"]},
        'periodSeconds': 5}
    return [game, editor, web, gateway, trainer, nginx, release]


def main():
    config = {'apiVersion': 'v1', 'kind': 'ConfigMap',
              'metadata': {'name': 'savsoul-runtime', 'namespace': 'savsoul'},
              'data': {name: (DEST / name).read_text() for name in ('nginx.conf', 'services.json')}}
    config['data']['launch.py'] = (ROOT / 'tools/kubernetes/launch.py').read_text()
    for name in ('release_server.py', 'release_transaction.py'):
        config['data'][name] = (ROOT / 'tools/kubernetes' / name).read_text()
    config['data']['receive_release.py'] = (ROOT / 'tools/server-host/receive_release.py').read_text()
    apps = containers()
    pod = {'automountServiceAccountToken': False, 'shareProcessNamespace': True,
           'terminationGracePeriodSeconds': 60,
           'securityContext': {'runAsNonRoot': True, 'runAsUser': 1009, 'runAsGroup': 1011,
                               'fsGroup': 1011, 'fsGroupChangePolicy': 'OnRootMismatch',
                               'seccompProfile': {'type': 'RuntimeDefault'}},
           'containers': apps,
           'volumes': [{'name': 'home', 'persistentVolumeClaim': {'claimName': 'savsoul-home'}},
                       {'name': 'config', 'configMap': {'name': 'savsoul-runtime'}},
                       {'name': 'release-auth', 'secret': {'secretName': 'savsoul-release-auth', 'defaultMode': 0o440}},
                       {'name': 'tls', 'secret': {'secretName': 'savsoul-tls', 'defaultMode': 0o440,
                        'items': [{'key': 'tls.crt', 'path': 'server.crt'},
                                  {'key': 'tls.key', 'path': 'server.key'}]}}]
                      + [{'name': app['name'] + '-tmp', 'emptyDir': {'sizeLimit': '64Mi'}} for app in apps]}
    stateful = {'apiVersion': 'apps/v1', 'kind': 'StatefulSet',
                'metadata': {'name': 'savsoul', 'namespace': 'savsoul'},
                'spec': {'serviceName': 'savsoul-internal', 'replicas': 1,
                         'updateStrategy': {'type': 'OnDelete'}, 'selector': {'matchLabels': LABEL},
                         'template': {'metadata': {'labels': LABEL}, 'spec': pod}}}
    policy = {'apiVersion': 'networking.k8s.io/v1', 'kind': 'NetworkPolicy',
              'metadata': {'name': 'savsoul-candidate', 'namespace': 'savsoul'},
              'spec': {'podSelector': {'matchLabels': LABEL}, 'policyTypes': ['Ingress', 'Egress'],
                       'ingress': [{'from': [{'ipBlock': {'cidr': '192.168.255.0/24'}}],
                                    'ports': [{'protocol': protocol, 'port': port} for protocol, port in
                                              [('TCP', 8080), ('TCP', 8443), ('TCP', 14000),
                                               ('TCP', 14001), ('TCP', 14080), ('TCP', 14481)]]}],
                       'egress': [{'to': [{'namespaceSelector': {'matchLabels': {'kubernetes.io/metadata.name': 'kube-system'}},
                                          'podSelector': {'matchLabels': {'k8s-app': 'kube-dns'}}}],
                                   'ports': [{'protocol': 'UDP', 'port': 53}, {'protocol': 'TCP', 'port': 53}]}]}}
    annotations = {'service.beta.kubernetes.io/linode-loadbalancer-default-protocol': 'tcp',
                   'service.beta.kubernetes.io/linode-loadbalancer-default-proxy-protocol': 'v1',
                   'service.beta.kubernetes.io/linode-loadbalancer-nodebalancer-type': 'common'}
    ports = [{'name': name, 'protocol': 'TCP', 'port': port, 'targetPort': target} for name, port, target in
             [('http', 80, 8080), ('https', 443, 8443), ('mud', 4000, 14000),
              ('mud-tls', 4001, 14001), ('editor-http', 4080, 14080), ('editor-tls', 4081, 14481)]]
    service = {'apiVersion': 'v1', 'kind': 'Service',
               'metadata': {'name': 'savsoul-public', 'namespace': 'savsoul', 'annotations': annotations},
               'spec': {'type': 'LoadBalancer', 'externalTrafficPolicy': 'Local', 'selector': LABEL, 'ports': ports}}
    headless = {'apiVersion': 'v1', 'kind': 'Service',
                'metadata': {'name': 'savsoul-internal', 'namespace': 'savsoul'},
                'spec': {'clusterIP': 'None', 'selector': LABEL, 'ports': [{'port': 4000}]}}
    (DEST / 'stack.json').write_text(json.dumps({'apiVersion': 'v1', 'kind': 'List',
                                               'items': [config, policy, headless, stateful, service]}, indent=2) + '\n')


if __name__ == '__main__':
    main()
