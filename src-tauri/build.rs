fn main() {
    #[cfg(target_os = "windows")]
    {
        // 仅正式(release)构建让应用以管理员身份运行：通过 UAC manifest 请求 requireAdministrator。
        // 这样打包部署后无论从注册表 Run 键 / 托盘 / 双击启动，都会静默提权为管理员。
        // 注意：开发(debug)构建不做提权，否则 `tauri dev` / `cargo run` 直接运行
        // 非提权 exe 会报「os error 740 请求的操作需要提升」，导致 `yarn start` 无法启动。
        let is_release = std::env::var("PROFILE").map(|p| p == "release").unwrap_or(false);
        let windows = tauri_build::WindowsAttributes::new();
        if is_release {
            // 自定义清单须额外声明 Common Controls v6 依赖，否则 tauri-plugin-dialog 的对话框会异常。
            let windows = windows.app_manifest(r#"
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="requireAdministrator" uiAccess="false" />
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#);
            tauri_build::try_build(
                tauri_build::Attributes::new().windows_attributes(windows),
            )
            .expect("failed to run build script");
        } else {
            tauri_build::build();
        }
        return;
    }
    #[cfg(not(target_os = "windows"))]
    tauri_build::build()
}
