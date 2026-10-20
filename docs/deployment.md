# Implantação

## Serviço e build

O executável `subdisc` contém API, gerenciamento de sessões e SFU WebRTC. Não há
processo de mídia separado obrigatório. `Dockerfile` compila o frontend e o
backend a partir de lockfiles e executa como usuário sem privilégios. GStreamer
é uma dependência do **cliente desktop**, não do servidor.

```bash
python3 scripts/configure.py --domain conversa.exemplo.com --public-ip 203.0.113.10
docker compose config --quiet
docker compose up --build -d
docker compose logs --tail=100 app caddy
```

O comando gera `.env` com permissão 0600 e uma chave de instalação aleatória;
recusa sobrescrever um arquivo existente. Configure DNS A para o endereço real.
No primeiro acesso, crie o administrador com `SETUP_KEY`. Não exponha a chave.

## Variáveis

| Variável | Função / padrão |
|---|---|
| `APP_ENV` | `development` para HTTP local; produção exige origem HTTPS |
| `APP_ORIGIN` | Origem exata, sem barra final; confere CSRF e upgrades |
| `APP_ADDRESS` | Listener/domínio do Caddy; local: `http://:8080` |
| `SETUP_KEY` | Segredo de instalação, mínimo 16 caracteres |
| `BIND` | API/SFU HTTP: `0.0.0.0:3000` |
| `DATABASE_URL` | SQLite; Compose usa `sqlite:///data/subdisc.db` |
| `STATIC_DIR` | Bundle web; Compose usa `/app/public` |
| `MEDIA_PUBLIC_IP` | IP público do host SFU em NAT 1:1; vazio para localhost/LAN |
| `MEDIA_UDP_MIN`, `MEDIA_UDP_MAX` | Faixa de candidatos ICE: 50000–51999 |
| `MEDIA_ICE_SERVERS` | Array JSON de servidores ICE; padrão `[]` |
| `RUST_LOG` | Filtro de logs; exemplo `subdisc=info`. `subdisc=debug` registra ofertas, estados de transporte e camadas recebidas |

O Compose publica a faixa UDP fixa acima. Se alterar seus limites, altere também
as publicações de portas e o firewall. Um domínio atende página, API e sinalização;
não é mais necessário domínio/porta HTTP separados para mídia. O backend valida
configuração JSON, IP e limites antes de iniciar.

## Portas, TLS e NAT

| Porta | Protocolo | Destino |
|---|---|---|
| 80 | TCP | Caddy: redirecionamento/ACME |
| 443 | TCP; UDP opcional para HTTP/3 | Caddy: HTTPS e WSS |
| 8080 | TCP | Apenas a configuração HTTP local |
| 50000–51999 | UDP | SFU Rust no contêiner app |
| 3478 | UDP/TCP, opcional | coturn |
| 5349 | TCP/TLS, opcional | coturn com certificados |
| 52000–52199 | UDP, opcional | Relays coturn |

Encaminhe as portas de mídia para o mesmo host e configure `MEDIA_PUBLIC_IP`
com o IP alcançável. Em rede local, deixar vazio preserva candidatos locais.
Endereços IPv6 link-local (`fe80::/10`) não são anunciados como candidatos.
Firewall/CGNAT ou redes que bloqueiam UDP precisam de TURN alcançável. Caddy é
proxy HTTP; não encaminha SRTP nem TURN. Mídia permanece criptografada com
DTLS/SRTP entre cada cliente e o SFU; o SFU tem acesso ao RTP para encaminhá-lo.

A seleção de portas é por transporte. O limite de 20 usuários é um controle de
admissão, não uma promessa de capacidade medida; planeje banda de saída em
função do número de assinantes e fontes de vídeo simultâneas.

## STUN/TURN opcional

Configure servidores ICE reais e autenticados:

```dotenv
MEDIA_ICE_SERVERS='[{"urls":["stun:turn.exemplo.com:3478","turn:turn.exemplo.com:3478?transport=udp","turns:turn.exemplo.com:5349?transport=tcp"],"username":"subdisc","credential":"senha-aleatoria"}]'
TURN_DOMAIN=turn.exemplo.com
TURN_USER=subdisc
TURN_PASSWORD=senha-aleatoria
```

O backend entrega a configuração somente após autenticar `join`. Credenciais TURN
são entregues aos participantes para alocar relays; use credenciais dedicadas e
rotacione-as. Elas não concedem acesso a salas ou à API. Sem TURN configurado,
não existe fallback oculto. Para coturn opcional:

```bash
docker compose -f docker-compose.yml -f infrastructure/compose.turn.yml up -d
```

Para TLS, forneça certificados válidos em `infrastructure/certs/fullchain.pem` e
`privkey.pem`, e acrescente `-f infrastructure/compose.turn-tls.yml`. Renove-os e
reinicie coturn para recarregar. Redes que permitem somente 443 precisam de
TURN/TLS em 443 num IP dedicado ou encaminhamento L4 apropriado; ajuste listener
e URL ICE juntos. Essa configuração ainda precisa de teste em rede externa.

## Desenvolvimento nativo

```bash
cp .env.example .env
npm --prefix frontend ci
./scripts/dev.sh
```

Vite atende 5173 e encaminha `/api`, inclusive WebSocket, para Rust/3000. Só Rust
e Vite são iniciados. Para servir o bundle diretamente, compile o frontend,
configure `STATIC_DIR`, `APP_ORIGIN`, `DATABASE_URL` e `SETUP_KEY`, e execute o
binário Rust. Use HTTPS atrás do Caddy em produção.

## Atualizar, monitorar e fazer backup

Use `scripts/backup.sh` para o SQLite e preserve o volume `app_data`. Pare a
aplicação para cópias manuais consistentes. Antes de atualizar, faça backup,
compile/teste e reinicie os containers. Reiniciar o backend encerra chamadas;
os clientes conservam contas e preferências e podem entrar novamente.

`GET /api/health` verifica acesso ao banco. Logs e estatísticas WebRTC ajudam a
separar falhas de sinalização, ICE e codecs. Não publique cookies, grants ou
SDP de ambientes reais. O perfil opcional `duckdns` permanece disponível;
configure `DUCKDNS_DOMAIN` e `DUCKDNS_TOKEN`. Mudança de IP público exige também
atualizar `MEDIA_PUBLIC_IP` e reiniciar o SFU, além da atualização do DNS.
