// Linux cloud integration check. Agent and recognizer processes are deterministic
// fixtures; ffmpeg, HTTP handlers, persistence and the browser UI are real.
const { chromium } = require('playwright');
const { spawn } = require('node:child_process');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');

(async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'recorder-ui-'));
  const whisper = path.join(root, 'whisper');
  const codex = path.join(root, 'codex');
  await fs.writeFile(whisper, `#!/bin/sh
if [ "$1" = "--help" ]; then exit 0; fi
while [ "$#" -gt 0 ]; do if [ "$1" = "-of" ]; then shift; output="$1"; fi; shift; done
printf '%s' 'Maya will send the launch proposal Friday. We approved the release.' > "$output.txt"
`, { mode:0o755 });
  await fs.writeFile(codex, `#!/bin/sh
if [ "$1" = "--version" ]; then echo 'codex smoke fixture'; exit 0; fi
if [ "$1" = "login" ]; then echo 'Logged in using ChatGPT'; exit 0; fi
while [ "$#" -gt 0 ]; do if [ "$1" = "--output-last-message" ]; then shift; output="$1"; fi; shift; done
cat > received-transcript.txt
printf '%s' '{"notes_markdown":"## Summary\\nThe release was approved.\\n\\n## Decisions\\n- Launch this week.","actions":[{"title":"Send launch proposal","owner":"Maya","due":"Friday"}]}' > "$output"
printf '%s\\n' '{"type":"item.completed","item":{"type":"agent_message","text":"Meeting notes generated"}}'
`, { mode:0o755 });
  await fs.writeFile(path.join(root,'model.bin'),'test-only fixture');
  const binary=path.resolve(__dirname,'../target/debug/openairecorder');
  const server=spawn(binary,[],{cwd:path.resolve(__dirname,'..'),env:{...process.env,RECORDER_DATA_DIR:path.join(root,'recordings'),WHISPER_BIN:whisper,WHISPER_MODEL:path.join(root,'model.bin'),CODEX_BIN:codex,CLAUDE_BIN:path.join(root,'missing-agent')}});
  let logs='';server.stdout.on('data',d=>logs+=d);server.stderr.on('data',d=>logs+=d);
  let browser;
  try {
    for(let i=0;i<100;i++){
      if(server.exitCode!==null)throw new Error(`Server exited: ${logs}`);
      if(logs.includes('is running'))break;
      await new Promise(r=>setTimeout(r,50));
    }
    browser=await chromium.launch({executablePath:process.env.CHROMIUM_BIN||'/usr/bin/chromium',args:['--no-sandbox','--use-fake-ui-for-media-stream','--use-fake-device-for-media-stream']});
    const page=await browser.newPage({viewport:{width:1360,height:960}});
    const errors=[];page.on('pageerror',e=>errors.push(e.message));
    await page.goto('http://127.0.0.1:8787');
    await page.locator('#title').fill('Product launch planning');
    // One second of PCM WAV; conversion really runs through ffmpeg.
    const wav=Buffer.alloc(44+32000);wav.write('RIFF');wav.writeUInt32LE(wav.length-8,4);wav.write('WAVE',8);wav.write('fmt ',12);wav.writeUInt32LE(16,16);wav.writeUInt16LE(1,20);wav.writeUInt16LE(1,22);wav.writeUInt32LE(16000,24);wav.writeUInt32LE(32000,28);wav.writeUInt16LE(2,32);wav.writeUInt16LE(16,34);wav.write('data',36);wav.writeUInt32LE(32000,40);
    await page.locator('#file').setInputFiles({name:'meeting.wav',mimeType:'audio/wav',buffer:wav});
    await page.locator('#detail-title').waitFor();assert.equal(await page.locator('#detail-title').textContent(),'Product launch planning');
    await page.waitForFunction(()=>document.getElementById('playback').readyState>=2);
    assert.equal(await page.locator('#playback').evaluate(a=>a.duration),1);
    await page.locator('#transcribe').click();
    await page.waitForFunction(()=>document.getElementById('transcript').textContent.includes('Maya will send'));
    await page.locator('#agent-consent').check();await page.locator('#generate-notes').click();
    await page.waitForFunction(()=>document.querySelector('#session-actions .task'));
    assert.match(await page.locator('#notes-preview').textContent(),/release was approved/);
    await page.locator('#pin').click();await page.waitForFunction(()=>document.getElementById('pin').textContent.includes('Unpin'));
    await page.locator('#session-actions input[type=checkbox]').check();
    await page.waitForFunction(()=>document.getElementById('action-count').textContent==='1/1 complete');
    await page.locator('#session-actions button').click();
    await page.waitForFunction(()=>document.querySelector('#session-actions button').textContent==='Copied');
    await page.locator('#global-nav').click();await page.locator('#global-list .task').waitFor();
    assert.equal(await page.locator('#global-list input[type=checkbox]').isChecked(),true);
    await page.locator('#global-list input[type=checkbox]').uncheck();
    await page.waitForFunction(()=>!document.querySelector('#global-list .task').classList.contains('done'));
    await page.locator('#search').fill('proposal');assert.equal(await page.locator('#library .library-item').count(),1);
    await page.locator('#search').fill('nonexistent phrase');assert.equal(await page.locator('#library .library-item').count(),0);
    await page.locator('#search').fill('');await page.locator('#library .library-item').click();
    assert.equal(await page.locator('#session-actions input[type=checkbox]').isChecked(),true);
    await page.screenshot({path:'/tmp/openairecorder-session.png',fullPage:true});
    page.once('dialog',d=>d.accept());await page.locator('#delete').click();await page.waitForFunction(()=>document.getElementById('count').textContent==='0');
    await page.locator('#global-nav').click();assert.equal(await page.locator('#global-list .task').count(),1);
    await page.locator('#new').click();await page.locator('#title').fill('Microphone capture test');
    await page.locator('input[name=source][value=mic]').check();await page.locator('#consent').check();await page.locator('#record').click();
    await page.locator('#record-widget').waitFor({state:'visible'});
    await page.waitForFunction(()=>document.getElementById('timer').textContent!=='00:00');
    await page.locator('#widget-stop').click();await page.waitForFunction(()=>document.getElementById('detail-title').textContent==='Microphone capture test');
    assert.equal(await page.evaluate(async()=>{const r=await fetch('/api/capture');return (await r.json()).active;}),false);
    await page.locator('#transcribe').click();await page.waitForFunction(()=>document.getElementById('transcript').textContent.includes('Maya will send'));
    await page.setViewportSize({width:390,height:844});await page.locator('#new').click();
    assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),true);
    await page.screenshot({path:'/tmp/openairecorder-mobile.png',fullPage:true});
    await page.locator('#settings-nav').click();
    await page.locator('input[name=theme][value=dark]').check();
    await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
    assert.equal(await page.evaluate(()=>getComputedStyle(document.documentElement).backgroundColor),'rgb(22, 33, 30)');
    await page.reload();await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
    await page.locator('#settings-nav').click();await page.locator('input[name=theme][value=light]').check();
    await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
    assert.equal(await page.evaluate(()=>getComputedStyle(document.documentElement).backgroundColor),'rgb(247, 248, 245)');
    await page.locator('input[name=theme][value=system]').check();await page.waitForFunction(()=>document.documentElement.dataset.theme==='system');
    await page.emulateMedia({colorScheme:'dark'});
    assert.equal(await page.evaluate(()=>getComputedStyle(document.documentElement).backgroundColor),'rgb(22, 33, 30)');
    await page.emulateMedia({colorScheme:'light'});
    assert.equal(await page.evaluate(()=>getComputedStyle(document.documentElement).backgroundColor),'rgb(247, 248, 245)');
    assert.deepEqual(errors,[]);
    console.log('PASS: import, ffmpeg conversion, mock recognition, subscription CLI contract, notes preview, session checkbox, pin, global copy, independent completion, search, deletion survival, mobile layout, browser microphone recording/stop, saved Light/Dark theme and live System changes; no browser errors.');
  } finally {
    if(browser)await browser.close();if(server.exitCode===null&&server.signalCode===null){server.kill('SIGTERM');await new Promise(r=>server.once('exit',r));}await fs.rm(root,{recursive:true,force:true});
  }
})().catch(e=>{console.error(e);process.exitCode=1;});
