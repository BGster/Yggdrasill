// 叙事日志区：按序渲染条目，新内容自动滚到底部。
import { useEffect, useRef } from "react";
import type { LogEntry } from "../session";

export function NarrativeLog({ entries }: { entries: LogEntry[] }) {
  const boxRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const box = boxRef.current;
    if (box) {
      box.scrollTop = box.scrollHeight;
    }
  }, [entries]);

  return (
    <div className="log" ref={boxRef} role="log" aria-live="polite" aria-label="叙事记录">
      {entries.length === 0 && <p className="log-empty">（故事尚未开始。）</p>}
      {entries.map((entry) => {
        switch (entry.kind) {
          case "narrative":
            return (
              <p key={entry.id} className="line-narrative">
                {entry.text}
              </p>
            );
          case "warning":
            return (
              <p key={entry.id} className="line-warning">
                ⚠ {entry.text}
              </p>
            );
          case "system":
            return (
              <p key={entry.id} className="line-system">
                {entry.text}
              </p>
            );
          case "error":
            return (
              <div key={entry.id} className="error-box" role="alert">
                <span className="error-code">[错误 {entry.code}]</span> {entry.message}
              </div>
            );
        }
      })}
    </div>
  );
}
