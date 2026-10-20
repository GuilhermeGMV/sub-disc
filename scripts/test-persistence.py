#!/usr/bin/env python3
"""Verifica dados e sessão após reiniciar o processo Rust real; usa banco descartável."""
import http.cookiejar,json,os,pathlib,socket,subprocess,tempfile,time,urllib.request
root=pathlib.Path(__file__).resolve().parent.parent
with tempfile.TemporaryDirectory(prefix='subdisc-persistence-') as tmp:
    with socket.socket() as sock:
        sock.bind(('127.0.0.1',0));port=sock.getsockname()[1]
    origin=f'http://127.0.0.1:{port}'
    env={**os.environ,'APP_ENV':'development','APP_ORIGIN':origin,'BIND':f'127.0.0.1:{port}',
         'DATABASE_URL':f'sqlite://{tmp}/test.db','SETUP_KEY':'persistence-setup-test-key',
         'MEDIA_ICE_SERVERS':'[]'}
    opener=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()))
    def call(path,data=None):
        request=urllib.request.Request(origin+'/api'+path,data=None if data is None else json.dumps(data).encode(),headers={'Origin':origin,'Content-Type':'application/json'})
        with opener.open(request,timeout=3) as response:return json.load(response)
    def start():
        proc=subprocess.Popen([os.environ.get('SUBDISC_BINARY',str(root/'backend/target/debug/subdisc'))],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        for _ in range(100):
            try:call('/health');return proc
            except OSError:
                if proc.poll() is not None:raise RuntimeError('Backend não iniciou')
                time.sleep(.05)
        proc.terminate();raise TimeoutError('Health check')
    proc=start()
    try:
        user=call('/setup',{'username':'admin','password':'strong password 123','setup_key':env['SETUP_KEY']})
        channels=call('/channels');text=next(c for c in channels if c['kind']=='text')
        message=call(f"/channels/{text['id']}/messages",{'content':'Persistência entre processos'})
        proc.terminate();proc.wait(timeout=8);proc=start()
        assert call('/me')==user
        assert call('/channels')==channels
        assert call(f"/channels/{text['id']}/messages")==[message]
        print('PASS: sessão, conta, canais e mensagens sobreviveram ao reinício do processo Rust.')
    finally:
        proc.terminate();proc.wait(timeout=8)
