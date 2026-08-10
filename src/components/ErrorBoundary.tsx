import React from "react";

interface State {
  error: Error | null;
}

/**
 * 全局错误边界：捕获渲染期异常，避免整页白屏。
 * 同时在此挂载 unhandledrejection 监听，统一上报未捕获的 Promise 异常。
 */
export class ErrorBoundary extends React.Component<React.PropsWithChildren, State> {
  constructor(props: React.PropsWithChildren) {
    super(props);
    this.state = { error: null };
  }

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[ErrorBoundary] 捕获到渲染错误:", error, info.componentStack);
  }

  componentDidMount() {
    window.addEventListener("unhandledrejection", this.handleRejection);
  }

  componentWillUnmount() {
    window.removeEventListener("unhandledrejection", this.handleRejection);
  }

  handleRejection = (e: PromiseRejectionEvent) => {
    console.error("[unhandledrejection]", e.reason);
  };

  handleReload = () => {
    this.setState({ error: null });
    window.location.reload();
  };

  render() {
    if (this.state.error) {
      return (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            height: "100vh",
            padding: 24,
            color: "#e5e7eb",
            background: "#0f172a",
            fontFamily: "system-ui, sans-serif",
          }}
        >
          <h2 style={{ marginTop: 0 }}>应用发生错误</h2>
          <pre
            style={{
              maxWidth: 640,
              maxHeight: 240,
              overflow: "auto",
              padding: 12,
              borderRadius: 8,
              background: "#1e293b",
              fontSize: 12,
              whiteSpace: "pre-wrap",
            }}
          >
            {this.state.error.message}
          </pre>
          <button
            onClick={this.handleReload}
            style={{
              marginTop: 16,
              padding: "8px 20px",
              borderRadius: 8,
              border: "none",
              background: "#3b82f6",
              color: "#fff",
              cursor: "pointer",
            }}
          >
            重新加载
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
