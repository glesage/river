const { chromium, expect } = require(require('path').resolve(__dirname,'../../../../../ui/tests/node_modules/@playwright/test'));
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const out = __dirname;
const t0 = Date.parse('2026-11-16T12:00:00Z');
const emojiList = ['🦀','❤️','👍','🎉','🔥','👀','🚀','💯','😀','🙌','✅','🤔'];
const urls = { before: process.argv[2], after: process.argv[3] };
const rows = process.env.RESUME && fs.existsSync(path.join(out,'manifest.json')) ? JSON.parse(fs.readFileSync(path.join(out,'manifest.json'))).scenarios : [];
const scenarios = [
  { id:'01-pills-desktop', title:'Pill appearance and selected counts', group:'Appearance', note:'The compact pill design, selected tint, and visible counts are retained.', run: async s=>{const r=await s.target('A quick reaction keeps the conversation moving.');await s.member('❤️');await s.member('🦀');await s.chip(r,'🦀').click();await s.settle();return r;} },
  { id:'02-keyboard-enter', title:'Enter inspects instead of reacting', group:'Activation', note:'Same Enter key: before adds your reaction; after opens the reactor list without changing the count.', run:async s=>{const r=await s.target('Who reacted to this message?');await s.member('🦀');await s.chip(r,'🦀').press('Enter');await s.settle();return r;} },
  { id:'03-keyboard-space-selected', title:'Space preserves your reaction while inspecting', group:'Activation', note:'Same Space key on your sole reaction: before removes it; after opens the explicit Remove action.', run:async s=>{const r=await s.target('Inspect my reaction without removing it.');await s.pick(r,'👍');await s.tick();await s.chip(r,'👍').press('Space');await s.settle();if(s.version==='before')await expect(s.chip(r,'👍')).toHaveCount(0);return r;} },
  { id:'04-list-react', title:'Explicit React action in the reactor list', group:'List actions', note:'Same Shift+Enter inspection gesture on desktop: the after list offers React with 🦀.', run:async s=>{const r=await s.target('React after checking who is here.');await s.member('🦀');await s.chip(r,'🦀').press('Shift+Enter');await s.settle();return r;} },
  { id:'05-list-switch', title:'Explicit Switch action in the reactor list', group:'List actions', note:'When you selected another emoji, the after list offers Switch to 🦀.', run:async s=>{const r=await s.target('Switch from my thumbs up to the crab.');await s.member('🦀');await s.pick(r,'👍');await s.chip(r,'🦀').press('Shift+Enter');await s.settle();return r;} },
  { id:'06-list-remove', title:'Explicit Remove remains available', group:'List actions', note:'Both versions show Remove for your own emoji; native pressed/expanded semantics are improved after.', run:async s=>{const r=await s.target('My reaction has an explicit remove action.');await s.pick(r,'👍');await s.chip(r,'👍').press('Shift+Enter');await s.settle();return r;} },
  { id:'07-phone-tap', title:'A phone tap inspects who reacted', group:'Mobile', width:390,touch:true, note:'Same real touch tap: before reacts immediately; after shows the names and explicit React action.', run:async s=>{const r=await s.target('A phone tap should tell me who reacted.');await s.member('🦀');await s.chip(r,'🦀').tap();await s.settle();return r;} },
  { id:'08-phone-tap-selected', title:'A phone tap no longer removes your reaction', group:'Mobile', width:390,touch:true, note:'Same real tap on your sole chip: before removes it; after opens the list with Remove.', run:async s=>{const r=await s.target('Keep my reaction until I choose Remove.');await s.pick(r,'👍');await s.tick();await s.chip(r,'👍').tap();await s.settle();if(s.version==='before')await expect(s.chip(r,'👍')).toHaveCount(0);return r;} },
  { id:'09-tablet-touch', title:'Touch inspection also works at tablet width', group:'Mobile', width:1024,touch:true, note:'Touch policy is independent of screen width. The tablet tap now inspects too.', run:async s=>{const r=await s.target('A short tap on a wider touch screen.');await s.member('🦀');await s.chip(r,'🦀').tap();await s.settle();return r;} },
  { id:'10-standing-order', title:'Earliest standing reaction determines emoji order', group:'Ordering', note:'Identical timed actions: 🦀 first, then 🔥, then ❤️. Before sorts alphabetically; after retains chronological standing order.', run:async s=>{const r=await s.target('Reaction order follows the conversation.');await s.member('🦀');await s.pick(r,'🔥');await s.member('❤️');return r;} },
  { id:'11-remove-readd-order', title:'Remove and re-add starts a new interval', group:'Ordering', note:'After removing and re-adding 🔥, it moves behind the still-standing 🦀 and ❤️. Before stays alphabetical.', run:async s=>{const r=await s.target('Re-adding a reaction gives it a new place.');await s.member('🦀');await s.pick(r,'🔥');await s.member('❤️');await s.tick();await s.chip(r,'🔥').click();await s.settle();await expect(s.chip(r,'🔥')).toHaveCount(0);await s.pick(r,'🔥');return r;} },
  { id:'12-keyboard-focus', title:'Visible native-button keyboard focus', group:'Accessibility', note:'Focus ring on the same chip. The accompanying DOM facts show the native button and descriptive accessible name.', run:async s=>{const r=await s.target('A clearly focused reaction chip.');await s.member('🦀');await s.page.keyboard.press('Tab');await s.chip(r,'🦀').focus();await s.page.keyboard.press('Shift');return r;}, keepFocus:true },
  { id:'13-direct-removal-focus', title:'Direct last-chip removal returns focus to React', group:'Accessibility', note:'Same mouse removal, then Shift to reveal the focus position. The after version leaves a reachable persistent React control.', run:async s=>{const r=await s.target('Removing my last reaction keeps an action in reach.');await s.pick(r,'👍');await s.tick();await s.page.evaluate(()=>document.activeElement?.blur());await s.chip(r,'👍').click();await s.settle();await expect(s.chip(r,'👍')).toHaveCount(0);await s.page.keyboard.press('Shift');return r;}, keepFocus:true },
  { id:'14-list-handoff', title:'Only the latest reactor list owns inspection', group:'Lifecycle', note:'Keyboard inspection moves between two author groups. Before can leave both lists open; after closes the old list and focuses the latest one.', tall: true, run:async s=>{await s.tick();await s.hook('deliverTimedMessages',0,[{text:'First author: inspect these reactions.',sentSecs:0,receivedSecs:0}]);const one=s.row('First author: inspect these reactions.');await expect(one).toBeVisible();await s.member('🦀');await s.tick();await s.hook('deliverTimedMessages',1,[{text:'Second author: move inspection here.',sentSecs:0,receivedSecs:0}]);const two=s.row('Second author: move inspection here.');await expect(two).toBeVisible();await s.member('❤️');await s.chip(one,'🦀').press('Shift+Enter');await s.settle();await s.chip(two,'❤️').press('Shift+Enter');await s.settle();return [one,two];} },
  { id:'15-dense-phone-light', title:'320px received-message wrapping in light mode', group:'Layout',width:320,touch:true, note:'Twelve chips still wrap inside the message column with one reachable React control.', run:async s=>{const r=await s.target('Twelve reactions in a narrow conversation.');await s.hook('addReactionsToNewest',emojiList);await expect(r.getByTestId('reaction-chip')).toHaveCount(12);return r;} },
  { id:'16-dense-phone-dark', title:'320px received-message wrapping in dark mode', group:'Layout',width:320,touch:true,scheme:'dark',note:'The existing dark palette, count contrast, and chip sizes are preserved.', run:async s=>{const r=await s.target('Twelve reactions in a narrow conversation.');await s.hook('addReactionsToNewest',emojiList);await expect(r.getByTestId('reaction-chip')).toHaveCount(12);return r;} },
  { id:'17-dark-switch', title:'The explicit Switch action in dark mode', group:'List actions',scheme:'dark',note:'The new list action uses the normal theme text and surface tokens.',run:async s=>{const r=await s.target('Switch after checking the reactor list.');await s.member('🦀');await s.pick(r,'👍');await s.chip(r,'🦀').press('Shift+Enter');await s.settle();return r;} },
  { id:'18-own-dense-phone', title:'320px own-message wrapping', group:'Layout',width:320,touch:true,note:'Your own row keeps its tint and footer geometry with dense native reaction buttons.',run:async s=>{const r=await s.own('My message keeps its tint and readable footer.');await s.hook('addReactionsToNewest',emojiList);await expect(r.getByTestId('reaction-chip')).toHaveCount(12);return r;} },
  { id:'19-pending-order',title:'Optimistic order while signing is pending',group:'Ordering',note:'Signing is held after the pick: the after UI appends the pending 🔥 without disturbing standing 🦀 and ❤️.',run:async s=>{const r=await s.target('A pending reaction keeps the standing order.');await s.member('🦀');await s.member('❤️');await s.hook('holdMessageSigning');await s.pick(r,'🔥');return r;} },
];

async function main(){
 const browser=await chromium.launch();
 const facts={commits:{before:'fc92f19bd64bed0860fa7f0b6864333a9bd1149b',after:'b5ea6bcbce5125317a29402e8d670f8f598a8e38'},fixedClock:new Date(t0).toISOString(),browser:browser.version(),scenarios:rows};
 for(const scenario of scenarios){
  if(rows.some(r=>r.id===scenario.id))continue;
  const record={id:scenario.id,title:scenario.title,group:scenario.group,note:scenario.note,versions:{}};
  for(const version of ['before','after']){
   console.log(`${scenario.id} ${version}`);
   const width=scenario.width||1280, height=scenario.tall?1000:800;
   const context=await browser.newContext({viewport:{width,height},hasTouch:!!scenario.touch,colorScheme:scenario.scheme||'light',locale:'en-US',timezoneId:'UTC',deviceScaleFactor:1});
   const page=await context.newPage();
   const errors=[];page.on('pageerror',e=>errors.push(e.message));
   await page.clock.setFixedTime(new Date(t0));
   await page.addInitScript(()=>{
    let entropy=0x92731a43, math=0x43716245;
    function next(){entropy^=entropy<<13;entropy^=entropy>>>17;entropy^=entropy<<5;return entropy>>>0;}
    Object.defineProperty(Crypto.prototype,'getRandomValues',{configurable:true,value:function(a){const bytes=new Uint8Array(a.buffer,a.byteOffset,a.byteLength);for(let i=0;i<bytes.length;i++)bytes[i]=next()&255;return a;}});
    Math.random=()=>{math^=math<<13;math^=math>>>17;math^=math<<5;return(math>>>0)/4294967296;};
   });
   await page.goto(urls[version]);
   await page.waitForSelector('.app-root');
   await page.waitForFunction(()=>{const links=[...document.querySelectorAll('link[rel="stylesheet"]')];return links.length&&links.every(l=>l.sheet);});
   const roomButton=page.getByTestId('room-list').getByRole('button',{name:'Your Private Room'});
   if(!(await roomButton.isVisible())){
    const menu=page.getByTestId('hamburger-rooms-button').filter({visible:true});
    if(await menu.count())await menu.click();
    else await page.setViewportSize({width:1280,height});
   }
   await roomButton.click();
   if(page.viewportSize().width!==width)await page.setViewportSize({width,height});
   await page.getByTestId('message-composer').waitFor();
   let ticks=0;
   const s={page,version,
    row:text=>page.getByTestId('message-row').filter({hasText:text}),
    chip:(r,emoji)=>r.getByTestId('reaction-chip').filter({hasText:emoji}),
    settle:async()=>{await page.evaluate(()=>new Promise(r=>requestAnimationFrame(()=>requestAnimationFrame(r))));await page.waitForTimeout(180);},
    tick:async()=>{await page.clock.setFixedTime(new Date(t0+(++ticks)*2000));},
    hook:async(name,...args)=>{await page.evaluate(({name,args})=>window.__riverTest[name](...args),{name,args});await s.settle();},
    target:async text=>{await s.tick();await s.hook('appendMessage',text);const r=s.row(text);await expect(r).toBeVisible();return r;},
    own:async text=>{await s.tick();await page.getByTestId('message-input').fill(text);await page.getByTestId('message-input').press('Enter');const r=s.row(text);await expect(r).toBeVisible();await s.settle();return r;},
    member:async emoji=>{await s.tick();await s.hook('memberReactsToNewest',emoji);},
    pick:async(r,emoji)=>{await s.tick();await r.hover({position:{x:4,y:4}});await r.getByTestId('message-react-button').click();const choices=width<768?page.locator('.mobile-reaction-choice'):page.getByTestId('emoji-picker').getByRole('button');await expect(choices.first()).toBeVisible();if(width<768)await page.waitForTimeout(420);await choices.filter({hasText:new RegExp('^'+emoji+'$','u')}).click();await s.settle();await expect(s.chip(r,emoji)).toHaveCount(1);}
   };
   await s.hook('setAvatarSalt',17);await s.hook('removeMessages','');
   const target=await scenario.run(s);
   await page.mouse.move(0,0);
   await s.settle();
   await page.evaluate(()=>document.fonts.ready);
   const targetRows=Array.isArray(target)?target:[target];
   const rects=await Promise.all(targetRows.map(r=>r.boundingBox()));
   const x=Math.max(0,Math.floor(Math.min(...rects.map(r=>r.x))-12));
   const y=Math.max(0,Math.floor(Math.min(...rects.map(r=>r.y))-30));
   const right=Math.min(width,Math.ceil(Math.max(...rects.map(r=>r.x+r.width))+12));
   const clip={x,y,width:right-x,height:Math.min(scenario.tall?480:350,height-y-10)};
   const dir=path.join(out,scenario.id);fs.mkdirSync(dir,{recursive:true});
   await page.screenshot({path:path.join(dir,version+'-full.png'),animations:'disabled'});
   await page.screenshot({path:path.join(dir,version+'.png'),clip,animations:'disabled'});
   const metadata=await page.evaluate(()=>({
    chips:[...document.querySelectorAll('[data-testid="reaction-chip"]')].map(e=>({text:e.textContent.trim(),tag:e.tagName,role:e.getAttribute('role')||e.tagName.toLowerCase(),emoji:e.getAttribute('data-emoji'),pressed:e.getAttribute('aria-pressed'),expanded:e.getAttribute('aria-expanded'),label:e.getAttribute('aria-label'),title:e.getAttribute('title'),focused:e===document.activeElement})),
    lists:[...document.querySelectorAll('[data-testid="reaction-owners"]')].map(e=>({text:e.innerText,focused:e===document.activeElement})),
    focus:{testid:document.activeElement?.getAttribute('data-testid'),text:document.activeElement?.textContent?.trim().slice(0,70),tag:document.activeElement?.tagName},
    overflow:document.documentElement.scrollWidth>innerWidth,
    theme:matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light',
    pointer:matchMedia('(pointer: fine)').matches?'fine':'coarse'
   }));
   if(metadata.overflow)throw new Error('Unexpected horizontal overflow in '+scenario.id+' '+version);
   if(errors.length)throw new Error('Page error '+errors.join('\n'));
   record.versions[version]={image:`${scenario.id}/${version}.png`,full:`${scenario.id}/${version}-full.png`,viewport:{width,height},clip,...metadata};
   await context.close();
  }
  rows.push(record);fs.writeFileSync(path.join(out,'manifest.json'),JSON.stringify(facts,null,2));
 }
 await browser.close();
 console.log('Captured '+rows.length+' pairs.');
}
main().catch(e=>{console.error(e);process.exit(1)});
