import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import LauncherPanel from "./launcher/LauncherPanel";
import { toast } from "./shared/Toast";

/** The existing global hotkey/window now hosts the same unified search as the launcher. */
export default function FileSearchPopup() {
  const [focus, setFocus] = useState(0);
  useEffect(() => {
    let disposed = false;
    let off: (() => void) | undefined;
    void listen("file-search-focus", () => { if (!disposed) setFocus((value) => value + 1); })
      .then((unlisten) => { if (disposed) unlisten(); else off = unlisten; })
      .catch((error) => { if (!disposed) toast(String(error), "err"); });
    return () => { disposed = true; off?.(); };
  }, []);
  return <LauncherPanel searchOnly searchFocus={focus}
    onSearchClose={() => { void invoke("hide_file_search_popup").catch((error) => toast(String(error), "err")); }} />;
}
