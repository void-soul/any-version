import { describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { LauncherSearchOverlay } from "./LauncherSearchOverlay";
import { fileSearchRequest } from "./fileSearchScope";
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
      webSources={[]} classificationMap={new Map()}
      fileScope={{drive:"",filter:"all"}} setFileScope={noop} refreshFiles={noop} />);
    expect(html.match(/type="text"/g)).toHaveLength(1);
    expect(html).toContain("Sample launcher app");
    expect(html).toContain("sample.pdf");
    expect(html).toContain("fileSearch.manage");
    expect(html).not.toContain("launcher.searchFileTools");
  });

  it("scopes only disk requests and distinguishes results after filter changes", () => {
    const all = fileSearchRequest(" report ", {drive:"",filter:"all"});
    const docs = fileSearchRequest("report", {drive:"E",filter:"doc"});
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
