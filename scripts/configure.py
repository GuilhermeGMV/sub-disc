#!/usr/bin/env python3
"""Gera .env de produção com segredos aleatórios, sem sobrescrever arquivos."""
import argparse, ipaddress, pathlib, re, secrets
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--domain',required=True,help='Ex.: conversa.exemplo.com')
p.add_argument('--media-domain',required=True,help='Ex.: rtc.exemplo.com')
p.add_argument('--public-ip',required=True)
a=p.parse_args()
for domain in [a.domain,a.media_domain]:
    if not re.fullmatch(r'[a-zA-Z0-9](?:[a-zA-Z0-9.-]*[a-zA-Z0-9])?',domain):p.error('Domínio inválido: não inclua protocolo, porta ou caminho')
ipaddress.IPv4Address(a.public_ip)
path=pathlib.Path(__file__).resolve().parent.parent/'.env'
values=dict(APP_ENV='production',APP_ORIGIN=f'https://{a.domain}',APP_ADDRESS=a.domain,MEDIA_ADDRESS=a.media_domain,SETUP_KEY=secrets.token_hex(32),LIVEKIT_URL=f'wss://{a.media_domain}',LIVEKIT_API_KEY='subdisc',LIVEKIT_API_SECRET=secrets.token_hex(32),LIVEKIT_NODE_IP=a.public_ip,LIVEKIT_LOOPBACK='false',TURN_ENABLED='false',TURN_DOMAIN='')
with path.open('x') as f:
    path.chmod(0o600)
    f.write('\n'.join(f'{k}={v}' for k,v in values.items())+'\n')
print('Criado .env com permissão 0600. Consulte SETUP_KEY localmente para criar o administrador.')
