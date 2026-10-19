# Validação e critérios de entrega

Data de execução: 18/10/2026. Plataforma: Linux x86_64, Rust 1.99.1, Node 24.13.1, Chromium de testes 156 via Playwright, LiveKit 1.13.10. Não confundir execução nativa com validação de containers.

## Resultados executados

| Verificação | Resultado |
|---|---|
| Testes Rust | 7 passaram |
| Vitest | 3 passaram |
| Playwright, fluxo completo | 1 passou com LiveKit real e dois usuários |
| Reinício de processo | Sessão, conta, canais e mensagem preservados |
| Svelte/TypeScript | Zero erros e zero avisos |
| Clippy `--all-targets -D warnings` | Passou |
| Rust `cargo build --release --locked` | Passou; binário otimizado também passou por persistência e E2E |
| Vite produção | Passou; SDK separado com 532,29 kB (138,16 kB gzip) |
| Caddy 2.11.7 | Configuração válida |
| Compose básico e sobreposição TURN/TLS | Configuração válida |
| Config LiveKit renderizada pelo Compose | Comando oficial `ports` aceitou o arquivo |

O Vite emite um aviso de chunk acima de 500 kB para o SDK oficial de mídia; isso não é erro de build. O tamanho reportado não é consumo de RAM em execução.

## Automatizados

- Backend: `./scripts/cargo.sh test --manifest-path backend/Cargo.toml` — autenticação, instalação privada, Argon2id, logout, cookie seguro, expiração, CSRF, autorização, convite expirado/de uso único/consumo concorrente, CRUD, validação, paginação, limites, grants LiveKit e WebSocket real.
- Frontend: `npm --prefix frontend run check` e `npm --prefix frontend test` — tipos Svelte/TypeScript, versão de eventos, rejeição de mensagens inválidas e reconciliação sem duplicatas.
- E2E: `npm --prefix frontend run test:e2e` — backend, SQLite e SFU reais; dois contextos independentes no navegador. Testa instalação, convite, login, mensagens bidirecionais, escape de conteúdo, recuperação de histórico após offline, sala compartilhada, recebimento de pacotes de áudio em ambos os clientes, câmera e tela remotas, mute/deafen, bloqueio da segunda aba, saída e layout mobile. Também verifica reprodução de áudio durante navegação para o chat.
- Persistência de processo: `python3 scripts/test-persistence.py` — encerra e inicia o executável Rust e verifica a mesma sessão, usuários, canais e mensagem no SQLite.
- Builds: Cargo e Vite de produção; lint Rust com Clippy; Caddyfile validado pelo Caddy 2.11.7; Compose validado com `config --quiet`.

O E2E não substitui uma avaliação auditiva: mede `inbound-rtp.bytesReceived`, `totalAudioEnergy > 0` nos dois clientes, publicação/subscrição e reprodução de elementos de áudio. Usa captura sintética do Chromium; não verifica qualidade percebida, eco físico ou áudio de compartilhamento em cada sistema operacional. Captura de tela sintética passou pelo fluxo real `getDisplayMedia`, sem substituir o LiveKit por um mock.

Para reproduzir E2E:

```bash
./scripts/cargo.sh build --manifest-path backend/Cargo.toml
npm --prefix frontend ci
npm --prefix frontend run build
cd frontend
npx playwright install chromium
LIVEKIT_BINARY=/caminho/livekit-server npm run test:e2e
```

Se usou um diretório local de browsers:

```bash
PLAYWRIGHT_BROWSERS_PATH="$PWD/../.tools/browsers" \
  LIVEKIT_BINARY="$PWD/../.tools/livekit/livekit-server" npm run test:e2e
```

Os testes precisam abrir sockets locais. No sandbox desta sessão, isso exigiu execução autorizada fora da restrição de sockets. O runner usa portas 18080 (app), 17880 (SFU), 17881/TCP e 17882/UDP. Banco e logs ficam em `/tmp/subdisc-e2e.*`; screenshots e traces em `frontend/test-results/`. O runner encerra os processos ao terminar. O teste usa uma instância limpa e não toca no banco de desenvolvimento ou produção.

## Checklist funcional

| Requisito | Implementação / evidência |
|---|---|
| Servidor Rust e migrations | Build, inicialização real e testes HTTP |
| Administrador, login, logout e convite | Testes Rust + E2E |
| Sessões e chat persistentes | SQLite WAL + teste de reinício de processo |
| CRUD de canais e autorização | Testes Rust; UI real de administração |
| Mensagens em tempo real e paginação | Rust + E2E; recuperação após desconexão |
| Presença online/offline e chamada | WebSocket autenticado e webhook assinado |
| Áudio entre dois usuários | Pacotes RTP recebidos nos dois navegadores |
| Webcam e tela ao vivo | Tracks reais recebidas no segundo navegador |
| Microfone, deafen e desconexão | E2E e estados remotos |
| Volume individual, dispositivos e qualidade | Implementado; requer checagem manual física |
| Canais vazios após reiniciar | Teste de persistência |
| Docker Compose em execução | **Executado: app, LiveKit e Caddy saudáveis; HTTP 200 na página, API e SFU** |
| HTTPS público, NAT, TURN/TLS | **Pendente: exige domínio e rede reais** |
| Linux ARM64 | **Pendente: não executado nesta máquina x86_64** |

Atualização da instalação local: o contexto `desktop-linux` passou a estar disponível. Foram corrigidas credenciais inválidas no `.env` (com cópia privada do anterior), e `docker compose up -d --wait` concluiu com os três serviços saudáveis. Página e `/api/health` em `http://localhost:8080`, `/api/setup` e sinalização em `http://localhost:7880` responderam HTTP 200. O E2E de mídia descrito acima continua sendo da execução nativa; ainda não foi repetido através da rede Docker.

A entrega permanece **parcialmente concluída** quanto aos critérios globais, mesmo com o fluxo nativo funcional. Os health checks do Compose confirmam a inicialização local, mas não comprovam mídia através da rede Docker nem conectividade pela internet.

## Validações manuais necessárias

1. Docker: `cp .env.example .env`, `docker compose up --build -d`, aguarde os três serviços saudáveis. Repita o fluxo de dois usuários em `http://localhost:8080`. Execute `docker compose restart app` e confirme login, histórico e canais. Use a chave inicial somente enquanto o banco estiver vazio.
2. Rede externa: use dois computadores em redes diferentes, um deles em rede móvel. Confirme o candidate pair em `chrome://webrtc-internals`. Teste UDP disponível, UDP bloqueado com ICE/TCP e, separadamente, TURN/TLS. Verifique certificados e origem HTTPS.
3. Áudio real: headphones, conversa simultânea, mute e deafen sem atraso perceptível, controle individual de volume, troca de entrada/saída, desconectar/reconectar USB/Bluetooth. Compare eco e ruído com suporte do navegador.
4. Webcam: ligar/desligar repetidamente, alterar qualidade, selecionar outro dispositivo, verificar grade e ampliação no outro cliente. Confirmar que áudio permanece estável.
5. Tela: compartilhar aba/janela/monitor, selecionar áudio onde oferecido pelo navegador. Parar pelo botão do navegador e verificar que a publicação e o indicador encerram. Repetir no Chromium e Firefox; Safari/mobile têm restrições próprias.
6. Reconexão: desligar a rede por alguns segundos durante a chamada, verificar recuperação SDK; testar queda longa com feedback e nova entrada. Reiniciar o backend e voltar à chamada. Fechar a aba abruptamente e conferir a saída no outro navegador.
7. Acessibilidade/mobile: navegação por teclado, leitor de tela, zoom 200%, viewport estreito, permissão negada de microfone, dispositivo ausente, falha de reprodução automática.
8. Backup: executar `scripts/backup.sh`, restaurar em instância de teste separada e comparar mensagens/canais. Não validar restauração sobre a única cópia de produção.
9. Capacidade: repetir com 5/10/20 usuários, registrar hardware e consumo conforme `docs/deployment.md`. O limite 20 por sala não é uma medição de capacidade do hardware.
