# Testes da mídia Rust

Execuções em Linux x86_64, 19/10/2026. Backend e SFU são o processo Rust real;
nenhum servidor ou SDK de mídia anterior foi iniciado pelos testes.

| Componente | Resultado executado |
|---|---|
| Backend Rust | 6 testes do SFU e 8 testes de API aprovados |
| Clippy backend | Aprovado com `--all-targets -- -D warnings` |
| Frontend | Svelte/TypeScript sem erros/avisos; 6 testes unitários aprovados; build aprovado |
| Simulcast/dynacast | Camada `h` em bloco grande, `m` em viewport de celular, volta a `h`; codificador sem camadas ativas com vídeo fora da tela e retomada |
| E2E completo em release | Dois cenários aprovados: dois navegadores e navegador ↔ desktop |
| Som da tela | Fonte estéreo sintética decodificada; mute e volume independentes; captura sem áudio também validada |
| Falhas transitórias na renegociação | Duas ofertas recusadas, depois recuperação do vídeo com bitrate preservado |
| Build release servidor | Aprovado com `--release --locked -j 2` |
| Fluxo contínuo X11: 120 s, republicação aos 60 s | Aprovado; vídeo retomado e 5.922 buffers de áudio / 2.361 frames decodificados |
| Interoperabilidade nativa repetida | 15/15 após filtrar IPv6 link-local do ICE; antes ~1 em 5 falhava com transporte preso em “conectando” |
| Desktop | 16 testes aprovados, Clippy e release aprovados; 2 integrações opt-in |
| Dois atores nativos | Áudio remoto, mute/deafen, volumes, recriação de áudio e encerramento aprovados |
| Captura X11 opt-in | Aprovada em Xvfb, inclusive destruição da janela e liberação dos objetos |
| Janela sintética pelo SFU | Aprovada; navegador reproduz pixels capturados e desktop recebe áudio/vídeo |
| Persistência após restart | Conta, sessão, canais e mensagens preservados |
| Compose com TURN/TLS | Validação de sintaxe aprovada; runtime não disponível |

## Backend e frontend

```bash
./scripts/cargo.sh test --locked --manifest-path backend/Cargo.toml
./scripts/cargo.sh clippy --locked --all-targets --manifest-path backend/Cargo.toml -- -D warnings
./scripts/cargo.sh build --locked --manifest-path backend/Cargo.toml
npm --prefix frontend run check
npm --prefix frontend test
npm --prefix frontend run build
npm --prefix frontend exec -- playwright install chromium
npm --prefix frontend run test:e2e
python3 scripts/test-persistence.py
```

Os testes de API mantêm autorização, Argon2id, instalação privada, convites,
cookies, expiração, CSRF, CRUD, paginação, limites e WebSocket reais. Os novos
testes verificam grant vinculado ao cookie, lease exclusivo, token inválido,
Origin inválida, SDP rejeitado sem reserva vazada, fonte exclusiva, troca de
socket com mesmo lease, revogação no logout e fechamento de transportes.

O E2E usa dois contextos Chromium com mídia sintética e backend/SFU reais.
Exige energia de áudio nos dois sentidos, vídeo decodificado, compartilhamento
de tela, indicadores, volumes, mute/deafen e exclusividade entre abas. Interrompe
o WebSocket e fecha os PeerConnections, depois exige nova negociação e recepção.
A reprodução de áudio continua quando o usuário volta ao chat. Para testar
recuperação, duas ofertas de publicação recebem HTTP 503 injetado pelo teste;
a oferta seguinte chega ao SFU real, e o teste exige vídeo novo decodificado e
o mesmo limite de bitrate.

Para simulcast, o teste exige as codificações `l`, `m` e `h` no emissor, uma única
camada ativa igual à largura decodificada no receptor (1280 → `h`, 640 → `m`),
a troca ao mudar o viewport do receptor para 390×844 e de volta, e nenhuma
camada ativa enquanto o receptor está no chat de texto. Ao voltar, exige frames
novos. Um teste do SFU reproduz o pacote do Chrome com extensões RTP e padding
que, antes da correção, encerrava o encaminhamento de vídeo entre navegadores.
Outros cobrem o descarte de sondas de padding com renumeração RTP e o filtro de
candidatos IPv6 link-local.

No compartilhamento, o teste acrescenta à captura uma faixa estéreo gerada por
Web Audio (440 Hz à esquerda, 880 Hz à direita). Opus, DTLS/SRTP, encaminhamento,
decodificação e reprodução usam o caminho normal. Verifica energia dessa faixa,
mute independente, volumes separados e sua remoção ao encerrar a tela. Outra
captura retorna somente vídeo e verifica a indicação “Sem áudio”. Isso valida
o transporte e os controles, não a captura de som físico de cada sistema.

## Interoperabilidade nativa

Na pasta do cliente desktop:

```bash
scripts/cargo.sh build --locked
. .deps/env.sh # somente com bibliotecas locais
SUBDISC_NATIVE_BINARY="$PWD/target/debug/subdisc-client" \
  npm --prefix ../sub-disc/frontend run test:e2e
python3 scripts/test-media-x11.py
SUBDISC_INTEROP_SECONDS=120 SUBDISC_PROBE_REPUBLISH_AT=60 \
  python3 scripts/test-media-x11.py
```

Sem `SUBDISC_NATIVE_BINARY`, o cenário nativo aparece explicitamente como
ignorado. Com a variável, o Playwright inicia o executável real, cria contas
com convite e verifica pixels reproduzidos no Chromium, buffers de áudio e
frames decodificados no desktop. O runner X11 cria Xvfb e janela próprios e
transmite pixels capturados por requisições X11 em Rust, em vez de videotestsrc.
O teste prolongado permite exigir uma segunda publicação da mesma fonte.
Os dois cenários completos e a rodada de dois minutos passaram com os binários
release de ambos os repositórios. A contagem da tabela corresponde a essa rodada.
O teste opt-in dos atores nativos está descrito na validação do cliente.

Na execução curta completa em release, o desktop decodificou 1.196 buffers de áudio e
473 frames; na execução com janela X11, 1.184 e 466. Os números variam com a
sincronização inicial. O teste exige progresso real, não um número fixo.
RTT local observado: 0–1 ms; buffer de áudio: aproximadamente 20–42 ms em
amostras distintas. Esses valores não medem latência total captura → tela.

## Limitações

Docker Desktop sem socket e daemon alternativo com acesso negado impediram
build/runtime de containers. Não foram validados Windows, portal Wayland,
TURN/TLS pela internet, dispositivos físicos, qualidade acústica ou carga de
20 usuários. Testes curtos e uma execução de dois minutos não comprovam
estabilidade por horas/dias. A adaptação simulcast foi validada com mudança de
tamanho e visibilidade; perda de pacotes real/controlada não foi injetada.

Detalhes, evidências e limites do desktop:
[docs/validation.md do cliente](../../sub-disc-client/docs/validation.md).
