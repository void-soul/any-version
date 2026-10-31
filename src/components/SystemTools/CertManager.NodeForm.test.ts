import { describe, expect, it } from "vitest";
import { formatIssueResultSummary, makeNodeConfig, NODE_TYPE_LABELS, visibleNodeFields } from "./CertManager.model";

describe("certificate issue result summary", () => {
  it("reports deployment failure separately from successful issuance", () => {
    expect(formatIssueResultSummary({ deployment: { results: [], ok_count: 1, fail_count: 2 } }, "issued"))
      .toBe("证书已签发，但 2 个部署失败，请查看部署日志");
  });
  it("retains normal messages for successful, absent, or empty deployments", () => {
    expect(formatIssueResultSummary({}, "issued")).toBe("issued");
    expect(formatIssueResultSummary({ deployment: { results: [], ok_count: 0, fail_count: 0 } }, "issued")).toBe("issued");
    expect(formatIssueResultSummary({ deployment: { results: [], ok_count: 2, fail_count: 0 } }, "issued")).toBe("issued");
  });
});

describe("local certificate deployment form contract", () => {
  it("initializes explicit backend defaults without HTTP credentials", () => {
    const config = makeNodeConfig("local");
    expect(config).toMatchObject({ output_dir: "", layout: "cert", reload_mode: "none" });
    expect(config).not.toHaveProperty("url");
    expect(config).not.toHaveProperty("token");
    expect(NODE_TYPE_LABELS.local).toBe("本机部署");
  });

  it("round trips all local settings and preserves legacy post_script", () => {
    const config = {
      output_dir: "/srv/certs", layout: "both", reload_mode: "nginx",
      reload_command: "", nginx_prefix: "/srv/nginx", nginx_conf: "conf/nginx.conf",
      service_name: "", post_script: "legacy-hook",
    };
    expect(makeNodeConfig("local", config)).toEqual(config);
    expect(makeNodeConfig("local")).not.toHaveProperty("post_script");
    expect(makeNodeConfig("windows", config)).toEqual({ url: "", token: "" });
  });

  it.each([
    ["none", []], ["command", ["reload_command"]],
    ["nginx", ["nginx_prefix", "nginx_conf"]], ["service", ["service_name"]],
  ])("shows only %s reload fields", (reload_mode, expected) => {
    const fields = visibleNodeFields("local", { ...makeNodeConfig("local"), reload_mode });
    expect(fields.map((field) => field.key)).toEqual(["output_dir", "layout", "reload_mode", ...expected]);
  });
});
