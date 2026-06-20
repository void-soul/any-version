import React, { useState, useEffect } from "react";
import Sidebar from "./components/Sidebar";
import EnvDiagnostics from "./components/EnvDiagnostics";
import SdkManager from "./components/SdkManager";
import ServiceManager from "./components/ServiceManager";
import CacheManager from "./components/CacheManager";
import MirrorManager from "./components/MirrorManager";
import PkgManager from "./components/PkgManager";
import SystemTools from "./components/SystemTools";
import GlobalSettings from "./components/GlobalSettings";
import { invoke } from "@tauri-apps/api/core";
import { ShieldCheck, Info } from "lucide-react";

import "./App.css";

export default function App() {
  const [activeTab, setActiveTab] = useState("diagnostics");

  // Call init command on mount to ensure directory structure and path env vars are initialized
  useEffect(() => {
    const initApp = async () => {
      try {
        // Rust config load handles the directory checks, but we could broadcast config init if needed.
        const conf = await invoke("get_config");
        console.log("AnyVersion initialized config: ", conf);
      } catch (e) {
        console.error("Initialization error: ", e);
      }
    };
    initApp();
  }, []);

  return (
    <div className="flex w-screen h-screen overflow-hidden bg-[#0d111d] text-slate-100 font-sans">
      {/* Sidebar Navigation */}
      <Sidebar activeTab={activeTab} setActiveTab={setActiveTab} />

      {/* Main Content Area */}
      <main className="flex-1 min-w-0 flex flex-col h-screen relative bg-gradient-to-br from-[#0d111d] via-[#101627] to-[#0a0e1a]">
        {/* Glow background effects */}
        <div className="absolute top-[-20%] left-[-10%] w-[50%] h-[50%] rounded-full bg-blue-600/5 blur-[120px] pointer-events-none"></div>
        <div className="absolute bottom-[-10%] right-[-10%] w-[60%] h-[60%] rounded-full bg-blue-500/5 blur-[150px] pointer-events-none"></div>

        {/* Dynamic page content */}
        <div className="flex-1 relative z-10">
          <div className={activeTab === "diagnostics" ? "h-full w-full" : "hidden"}>
            <EnvDiagnostics />
          </div>
          <div className={activeTab === "sdks" ? "h-full w-full" : "hidden"}>
            <SdkManager />
          </div>
          <div className={activeTab === "services" ? "h-full w-full" : "hidden"}>
            <ServiceManager />
          </div>
          <div className={activeTab === "caches" ? "h-full w-full" : "hidden"}>
            <CacheManager />
          </div>
          <div className={activeTab === "mirrors" ? "h-full w-full" : "hidden"}>
            <MirrorManager />
          </div>
          <div className={activeTab === "packages" ? "h-full w-full" : "hidden"}>
            <PkgManager />
          </div>
          <div className={activeTab === "tools" ? "h-full w-full" : "hidden"}>
            <SystemTools />
          </div>
          <div className={activeTab === "settings" ? "h-full w-full" : "hidden"}>
            <GlobalSettings />
          </div>
        </div>
      </main>
    </div>
  );
}
