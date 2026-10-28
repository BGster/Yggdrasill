// 工具栏：存档 / 读档 / 状态 / 重新开始（与 CLI 的 s / l / i / 重启同源）。
export function Toolbar({
  onStartable,
  onSave,
  onLoad,
  onState,
  onRestart,
}: {
  onStartable: boolean;
  onSave: () => void;
  onLoad: () => void;
  onState: () => void;
  onRestart: () => void;
}) {
  return (
    <div className="toolbar" role="toolbar" aria-label="游戏操作">
      <button type="button" className="tool-button" disabled={!onStartable} onClick={onSave}>
        存档
      </button>
      <button type="button" className="tool-button" disabled={!onStartable} onClick={onLoad}>
        读档
      </button>
      <button type="button" className="tool-button" disabled={!onStartable} onClick={onState}>
        状态
      </button>
      <button type="button" className="tool-button" onClick={onRestart}>
        重新开始
      </button>
    </div>
  );
}
