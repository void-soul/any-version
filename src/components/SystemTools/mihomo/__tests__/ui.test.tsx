import { expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { SettingItem, Toggle } from "../ui";
it("a setting switch is named by its visible setting title", () => {
  const html = renderToStaticMarkup(<SettingItem title="Enable DNS"><Toggle v={true} onChange={() => {}} /></SettingItem>);
  const title = html.match(/id="([^"]+)"/);
  expect(title).not.toBeNull();
  expect(html).toContain(`aria-labelledby="${title![1]}"`);
  expect(html).toContain('aria-checked="true"');
});
it("an explicit switch label takes precedence over a setting container", () => {
  const html = renderToStaticMarkup(<SettingItem title="Network"><Toggle label="Allow LAN" v={false} disabled onChange={() => {}} /></SettingItem>);
  expect(html).toContain('aria-label="Allow LAN"');
  expect(html).not.toContain('aria-labelledby=');
  expect(html).toContain('disabled=""');
});
