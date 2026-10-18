# Instalação Linux e operação

## Domínio e HTTPS

Use Linux x86_64 ou ARM64, Docker Engine e Compose recente. A imagem da aplicação compila Rust e Node para a arquitetura do host. Nenhum binário x86_64 baixado durante os testes é copiado para a imagem. ARM64 ainda requer validação real.

Crie dois registros DNS A apontando para o IPv4 público do servidor:

- `conversa.exemplo.com`: aplicação.
- `rtc.exemplo.com`: sinalização LiveKit.

Somente publique AAAA se o IPv6 e o firewall estiverem configurados. Serviços DNS com proxy HTTP precisam permitir WebSockets; prefira inicialmente DNS direto. Não é necessário conta de serviço de comunicação em nuvem.

Em uma instalação nova, sem `.env`:

```bash
python3 scripts/configure.py \
  --domain conversa.exemplo.com \
  --media-domain rtc.exemplo.com \
  --public-ip 203.0.113.10
# Substitua o IP de exemplo pelo endereço público real.
docker compose up --build -d
docker compose ps
docker compose logs --tail=100 app livekit caddy
```

O script gera segredos aleatórios, protege `.env` com modo 0600 e se recusa a sobrescrever um arquivo existente. Para converter uma instalação local, salve o `.env` anterior em local privado antes de executar. Não troque o volume do banco.

O Caddy obtém e renova certificados ACME automaticamente para os dois nomes. O acesso externo exige DNS válido, portas 80/443 liberadas e servidor alcançável. Abra `https://conversa.exemplo.com`, consulte a `SETUP_KEY` no `.env` local e crie a primeira conta. Depois da primeira conta, a rota de instalação fica bloqueada por transação no banco.

## Variáveis

| Variável | Significado |
|---|---|
| `APP_ENV` | `development` somente em localhost; demais valores usam proteção de produção |
| `APP_ORIGIN` | Origem exata do navegador, sem barra final; ex. `https://conversa.exemplo.com` |
| `APP_ADDRESS` | Endereço do site Caddy; domínio sem esquema habilita HTTPS automático |
| `MEDIA_ADDRESS` | Domínio Caddy da sinalização |
| `SETUP_KEY` | Chave secreta para o primeiro administrador; mínimo 16 caracteres |
| `LIVEKIT_URL` | URL pública WebSocket: `wss://rtc.exemplo.com` |
| `LIVEKIT_API_KEY` | Identificador da chave, não é senha; padrão `subdisc` |
| `LIVEKIT_API_SECRET` | Segredo compartilhado entre backend e SFU; produção exige pelo menos 32 caracteres |
| `LIVEKIT_NODE_IP` | IPv4 realmente alcançável anunciado pelo SFU aos clientes |
| `LIVEKIT_LOOPBACK` | `true` somente nos testes locais; `false` em produção |
| `TURN_ENABLED` | Liga TURN/UDP integrado, padrão `false` |
| `TURN_DOMAIN` | Nome DNS do TURN quando habilitado |
| `DATABASE_URL` | Definida no Compose: `sqlite:///data/subdisc.db` |
| `LIVEKIT_INTERNAL_URL` | Definida no Compose: `http://livekit:7880` |
| `STATIC_DIR`, `BIND`, `RUST_LOG` | Caminho do build SPA, socket HTTP e nível de logs do Rust |

Configuração do LiveKit no Compose é renderizada em `configs.livekit.content`. `infrastructure/livekit.yaml` é a referência para execução nativa local. O segredo chega ao LiveKit por `LIVEKIT_KEYS` e ao Rust por ambiente; nunca pelo bundle do frontend. `docker compose config` sem `--quiet` mostra variáveis resolvidas: não publique sua saída de produção.

## Portas e NAT

| Porta pública | Protocolo | Finalidade |
|---|---|---|
| 80 | TCP | ACME e redirecionamento HTTPS |
| 443 | TCP | Aplicação HTTPS e sinalização WSS pelo Caddy |
| 443 | UDP | HTTP/3 opcional do Caddy |
| 7881 | TCP | Transporte ICE/TCP alternativo |
| 7882 | UDP | ICE/UDP mux, tráfego WebRTC principal |
| 3478 | UDP | TURN/UDP integrado, somente se habilitado |
| 5349 | TCP | TURN/TLS opcional no exemplo abaixo |

As publicações 8080/7880 do Caddy são usadas pela configuração local. Em produção com domínios, o Caddy não abre esses listeners; ainda assim remova essas duas publicações se não precisar do modo local. As portas internas 3000 (Rust), 7880 (LiveKit) e 2019 (admin Caddy) não são publicadas diretamente.

Libere as portas no firewall do host, no firewall do provedor e no roteador. **UDP 7882 não passa pelo proxy HTTPS.** O projeto usa mux UDP; não exige abrir 50000–60000 ao mesmo tempo. Exemplo UFW: `sudo ufw allow 7882/udp`, `sudo ufw allow 7881/tcp`, além de HTTP/HTTPS. Regras Docker e UFW interagem: confirme de uma rede externa quais portas estão realmente acessíveis.

Atrás de NAT com IP público, encaminhe as portas para o host Docker e defina `LIVEKIT_NODE_IP` como o IP público. Teste também a partir de outra rede; NAT hairpin pode se comportar diferente na LAN. IP dinâmico exige atualização de DNS e do IP anunciado, seguida de reinício do LiveKit.

**CGNAT sem encaminhamento impede conexões de entrada.** HTTPS sozinho e TURN instalado na mesma rede inacessível não corrigem isso. Use servidor com IP público alcançável ou uma rede privada/VPN que transporte UDP e seja acessível a todos. Túneis HTTP comuns não transportam a mídia WebRTC necessária. Não há promessa de conectividade universal.

A [tabela oficial de portas do LiveKit](https://docs.livekit.io/transport/self-hosting/ports-firewall/) detalha os transportes e a diferença entre sinalização e mídia.

## TURN integrado

Para TURN/UDP, configure `TURN_ENABLED=true`, `TURN_DOMAIN=turn.exemplo.com`, DNS para o servidor e abra 3478/UDP. O cliente recebe as credenciais ICE do próprio LiveKit; não configure senha TURN no frontend. STUN também é servido pelo listener TURN/UDP. Sem TURN, a configuração usa os candidatos diretos do IP público anunciado.

Para TURN/TLS, obtenha um certificado público válido para `turn.exemplo.com` com seu cliente ACME, salve `fullchain.pem` e `privkey.pem` em `infrastructure/certs/` (fora do Git), proteja a chave e use:

```bash
docker compose -f docker-compose.yml -f infrastructure/compose.turn-tls.yml up -d
```

A sobreposição exige `TURN_DOMAIN`, habilita TLS no próprio LiveKit e publica 5349/TCP. O Caddy continua responsável somente por HTTPS/WSS; proxy HTTP comum não termina TURN. Renove os certificados com seu cliente ACME e reinicie o LiveKit para carregar os novos arquivos.

Redes corporativas que liberam apenas 443 podem bloquear 5349. Para atendê-las, use TURN/TLS em 443 com IP público dedicado ou proxy **L4** apropriado. Ajuste o listener TLS e a porta anunciada para 443; faça o Caddy escutar somente no outro IP. Não mapeie simplesmente 443 para 5349 sem corrigir a porta anunciada. Veja a [implantação oficial LiveKit com TURN/TLS](https://docs.livekit.io/transport/self-hosting/vm/). Não foi necessário acrescentar coturn.

## Persistência, backup e restauração

`app_data` contém SQLite e seus arquivos WAL/SHM; `caddy_data` mantém certificados e contas ACME; `caddy_config` mantém estado do proxy. `restart: unless-stopped` reinicia os serviços quando Docker reinicia. Habilite o daemon no boot: `sudo systemctl enable --now docker` quando aplicável à distribuição.

Para backup consistente e simples:

```bash
./scripts/backup.sh ./backups
```

O script para brevemente o backend, copia **todo** `/data`, incluindo WAL se presente, e reinicia o backend mesmo em caso de erro. O arquivo contém contas, hashes e sessões; proteja e guarde uma cópia fora da máquina. Guarde `.env` separadamente de forma segura. Não copie somente `subdisc.db` enquanto a aplicação escreve: dados recentes podem estar no WAL.

Restauração (sobrescreve os dados existentes; faça backup antes):

```bash
docker compose stop app
docker compose run --rm --no-deps --user root \
  -v "$PWD/backups:/backup:ro" --entrypoint sh app -c \
  'rm -f /data/subdisc.db /data/subdisc.db-wal /data/subdisc.db-shm && tar -xzf /backup/SEU-ARQUIVO.tar.gz -C /data && chown -R 10001:10001 /data'
docker compose start app
```

Use apenas um backup confiável produzido pelo script. Após restaurar, teste login e histórico. Canais vazios persistem no SQLite e não dependem do ciclo de vida das salas SFU.

Atualização:

```bash
./scripts/backup.sh
# Atualize o código e revise as notas de versão/migrations.
docker compose build --pull app
docker compose pull livekit caddy
docker compose up -d
docker compose ps
```

Nunca execute `docker compose down -v` para atualizar: `-v` remove os volumes. Migrations são aplicadas na inicialização. Rollback de migração exige restaurar o backup compatível; não é automático.

## Medir na máquina real

- `docker stats --no-stream`: memória, CPU e rede em repouso e com 2, 5, 10 e 20 participantes.
- `pidstat -r -u -p PID 1`: execução nativa; compare repouso, só áudio e compartilhamento.
- `iftop`/`nload` na interface de rede: banda do SFU; registre direção e duração da amostra.
- `chrome://webrtc-internals` ou `about:webrtc`: RTT, jitter, perda, bitrate e codec negociado. RTT não é latência de ponta a ponta de áudio.
- Estatísticas WebRTC e logs do LiveKit: reconexões e candidate pair selecionado. Não cole tokens/cookies em relatórios.

O teste local não é benchmark de produção. Não há números estimados de RAM, CPU ou capacidade garantida. O frontend gera um chunk separado para o SDK LiveKit; o build informa seu tamanho real.
