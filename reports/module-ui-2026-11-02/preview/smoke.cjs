const { chromium } = require(process.env.PLAYWRIGHT_MODULE || "playwright");
const fs = require("node:fs/promises");
const path = require("node:path");
const order = ["launcher","buddy","sdk","ai","mindmap","api","node","news","mihomo","cert","clipboard","otp","translate","picky","network","httpServer","rtspServer","imageBase64","env","logViewer","jsonBrowser","markdownReader","serial","wsDebugger","music","favorites","tasks","settings","suno","dbdesigner"];
(async () => {
 const output = path.resolve("reports/module-ui-2026-11-02/ui-evidence");
 await fs.mkdir(output,{recursive:true});
 const browser = await chromium.launch({channel:"msedge",headless:true});
 const context = await browser.newContext({viewport:{width:1280,height:800},locale:"zh-CN",timezoneId:"Asia/Shanghai"});
 await context.route("**/*", route => {
   const url = new URL(route.request().url());
   return url.origin === "http://127.0.0.1:5196" || ["blob:","data:"].includes(url.protocol) ? route.continue() : route.abort();
 });
 const results=[];
 const required={launcher:["launcher_get_settings"],buddy:["buddy_get_account_view","buddy_get_expiry_columns"],sdk:["project_list_fast"],ai:["get_provider_presets"],mindmap:["mm_list_folders"],api:["api_list_projects"],node:["npm_list_projects"],news:["get_rss_config"],mihomo:["mihomo_get_state"],cert:["deploy_node_list"],clipboard:["clipboard_get_ignored_apps"],otp:["otp_list_categories"],translate:["get_translate_config"],picky:["picky_get_state"],network:["net_traffic"],httpServer:["get_running_http_servers"],rtspServer:["get_rtsp_camera_devices"],env:["get_path_directories"],markdownReader:["take_pending_markdown_files"],serial:["serial_list_ports"],music:["music_get_library"],favorites:["fav_stats","fav_credential_status"],tasks:["tasks_list_range"],settings:["launcher_get_settings"],suno:["suno_list_profiles"],dbdesigner:["dbd_get_settings"]};
 const selected=process.env.ACCEPT_MODULES ? order.filter(id=>process.env.ACCEPT_MODULES.split(",").includes(id)) : order;
 if(process.env.ACCEPT_MODULES) results.push(...JSON.parse(await fs.readFile(path.join(output,"smoke.json"),"utf8").catch(()=>"[]")).filter(result=>!selected.includes(result.id)));
 try {
  for (let offset=0;offset<selected.length;offset+=3) {
  await Promise.all(selected.slice(offset,offset+3).map(async id => {
   const page=await context.newPage(), errors=[];
   page.on("pageerror",e=>errors.push(String(e)));
   page.on("console",msg=>{if(msg.type()==="error") errors.push(msg.text().slice(0,600));});
   try {
   await page.goto(`http://127.0.0.1:5196/reports/module-ui-2026-11-02/preview/index.html?module=${id}`,{waitUntil:"domcontentloaded"});
   const root=page.getByTestId("fixture-root");
   await root.waitFor({state:"visible"});
   await page.waitForFunction(() => {
     const root=document.querySelector('[data-testid="fixture-root"]');
     return root && root.innerText.trim() && root.innerText.trim() !== "加载中…";
   });
   await page.waitForFunction(commands => commands.every(command => (window.__fixtureCalls || []).some(call => call.command === command)), required[id] || []);
   if(id === "logViewer") await root.locator(".lv-root").waitFor();
   if(id === "jsonBrowser") {
    await page.waitForFunction(()=> !document.querySelector('[data-testid="fixture-root"]').innerText.includes("解析中"));
    await root.locator(".monaco-editor").first().waitFor({state:"visible",timeout:60000});
   }
   await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
   const text=await root.innerText();
   const calls=await page.evaluate(()=>window.__fixtureCalls || []);
   const overflow=await page.evaluate(()=>({width:document.documentElement.scrollWidth,viewport:innerWidth}));
   await page.screenshot({path:path.join(output,`${id}.png`),animations:"disabled"});
   await page.setViewportSize({width:800,height:900});
   await page.screenshot({path:path.join(output,`${id}-narrow.png`),animations:"disabled"});
   const missing=await page.evaluate(()=>window.__fixtureMissing || []);
   results.push({id,text:text.slice(0,2400),errors,calls,overflow,missing});

   console.log(id,errors.length,text.slice(0,60).replace(/\n/g," "));
   } catch(error) {
    results.push({id,errors,harnessError:String(error)});

    console.error(id,String(error));
   } finally { await page.close(); }
  }));
  results.sort((a,b)=>order.indexOf(a.id)-order.indexOf(b.id));
  await fs.writeFile(path.join(output,"smoke.json"),JSON.stringify(results,null,2));
  }
 } finally { await browser.close(); }
 if(results.some(r=>r.harnessError || r.errors.length || r.missing?.length || r.text === "加载中…")) process.exitCode=1;
})().catch(e=>{console.error(e);process.exitCode=1;});
