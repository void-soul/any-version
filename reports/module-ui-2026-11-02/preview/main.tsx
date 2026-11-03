import { Activity, useState, useEffect } from "react";
import { createRoot } from "react-dom/client";
import "../../../src/App.css";
import { initI18n } from "../../../src/i18n";
import { MODULE_MAP } from "../../../src/moduleRegistry";
import { ErrorBoundary } from "../../../src/components/ErrorBoundary";
import { calls } from "./tauri";
import { preapplyThemeAccent } from "../../../src/utils/brand";
const order = ["launcher","buddy","sdk","ai","mindmap","api","node","news","mihomo","cert","clipboard","otp","translate","picky","network","httpServer","rtspServer","imageBase64","env","logViewer","jsonBrowser","markdownReader","serial","wsDebugger","music","favorites","tasks","settings","suno","dbdesigner"];
function Preview() {
 const [index,setIndex] = useState(Math.max(0,order.indexOf(new URLSearchParams(location.search).get("module") || "launcher")));
 const [callCount,setCallCount]=useState(0);
 useEffect(()=>{const refresh=()=>setCallCount(calls.length);window.addEventListener("fixture-call",refresh);return()=>window.removeEventListener("fixture-call",refresh);},[]);
 const [visited,setVisited] = useState(()=>new Set([order[index]]));
 const id=order[index];
 useEffect(()=>setVisited(previous=>new Set([...previous,id])),[id]);
 return <div style={{height:"100vh",display:"flex",flexDirection:"column",background:"#10151f",color:"#cbd5e1"}}>
  <header style={{display:"flex",alignItems:"center",gap:12,padding:"8px 16px",borderBottom:"1px solid #334155",flexShrink:0,fontSize:12}}>
   <b>隔离界面验收 {index+1}/30</b>
   <select aria-label="验收模块" value={index} onChange={event=>{calls.length=0;setIndex(Number(event.target.value));}} style={{background:"#1e293b",color:"white",padding:6}}>{order.map((key,i)=><option key={key} value={i}>{i+1} {MODULE_MAP[key].label}</option>)}</select>
   <span>模拟数据 · 所有系统操作已隔离</span>
   <details style={{marginLeft:"auto"}}><summary>IPC记录 {callCount}</summary><pre style={{position:"absolute",zIndex:10000,right:8,top:44,background:"#10151f",maxHeight:240,overflow:"auto",maxWidth:640,padding:12}}>{JSON.stringify(calls.slice(-12),null,2)}</pre></details>
  </header>
  <main data-testid="fixture-root" style={{flex:1,minHeight:0,overflow:"hidden"}}>
   {[...new Set([...visited,id])].map(key=>{const Comp=MODULE_MAP[key].Component;return <Activity key={key} mode={key===id?"visible":"hidden"}><div className="h-full w-full flex flex-col"><ErrorBoundary><Comp selectedId={null} onSelectId={()=>{}} /></ErrorBoundary></div></Activity>;})}
  </main>
 </div>;
}
preapplyThemeAccent();
await initI18n("zh");
createRoot(document.getElementById("root")!).render(<Preview/>);
