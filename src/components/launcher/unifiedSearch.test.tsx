import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { LauncherSearchOverlay } from "./LauncherSearchOverlay";
import { fileSearchRequest, searchesFiles, searchesLauncher, webSearchUrl } from "./fileSearchScope";
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

describe("launcher unified search", () => {
  it("renders launcher items and disk hits beside a single search input", () => {
    const noop = () => {};
    const html = renderToStaticMarkup(<LauncherSearchOverlay
      searchInputRef={{ current: null }} searchQuery="sample" setSearchQuery={noop}
      setSearchSelectedIndex={noop} searchTotal={2} searchSelectedIndex={0}
      searchQuickActions={[]} launchVirtual={async () => {}} closeSearch={noop}
      searchResults={[{id:1,classificationId:1,name:"Sample launcher app",itemType:0,data:{target:"C:/sample.exe"},shortcutKey:null,globalShortcutKey:false,order:0}]}
      handleExecuteItem={async () => {}} effectiveFileHits={[{path:"D:/sample.pdf",name:"sample.pdf",drive:"D",isDir:false,size:123,modified:0}]}
      openFileHit={async () => true} fileQueryError="" fileSearchBusy={false}
      webSources={[{id:"test",name:"Example search",keyword:"ex",url:"https://example.test?q=%s"}]} classificationMap={new Map()}
      fileScope={{drive:"",filter:"all"}} setFileScope={noop} refreshFiles={noop} />);
    expect(html.match(/type="text"/g)).toHaveLength(1);
    expect(html).toContain("Sample launcher app");
    expect(html).toContain("sample.pdf");
    expect(html).toContain("launcher.searchSettings");
    expect(html).toContain("launcher.searchSource");
    expect(html).toContain("launcher.fileType");
    expect(html).toContain('value="web:test"');
    expect(html).toContain("Example search");
    expect(html).not.toContain("ex ·");
    expect(html).not.toContain("launcher.searchFileTools");
  });

  it("scopes only disk requests and distinguishes results after filter changes", () => {
    expect(webSearchUrl("https://example.test?q=%s", "中文 & #")).toBe("https://example.test?q=%E4%B8%AD%E6%96%87%20%26%20%23");
    expect(webSearchUrl("https://example.test?q=", "a b")).toBe("https://example.test?q=a%20b");
    const all = fileSearchRequest(" report ", {drive:"",filter:"all"});
    const docs = fileSearchRequest("report", {drive:"E",filter:"doc"});
    expect(searchesFiles({drive:"",filter:"all",source:"launcher"})).toBe(false);
    expect(searchesLauncher({drive:"",filter:"all",source:"files"})).toBe(false);
    expect(searchesLauncher({drive:"",filter:"all",source:"web:google"})).toBe(false);
    expect(searchesFiles({drive:"",filter:"all",source:"web:google"})).toBe(false);
    expect(all.options.query).toBe("report");
    expect(all.options.drives).toEqual([]);
    expect(docs.options.drives).toEqual(["E"]);
    expect(docs.options.kind).toBe("file");
    expect(docs.options.exts).toContain("pdf");
    expect(docs.key).not.toBe(all.key);
    expect(fileSearchRequest("other", {drive:"E",filter:"doc"}).key).not.toBe(docs.key);
    expect(fileSearchRequest("report", {drive:"",filter:"dir"}).options.kind).toBe("dir");
  });
});
