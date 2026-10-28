// 待决选择面：提示语 + 编号选项按钮（键盘 1-9 由 App 统一监听）。
import type { PendingChoice } from "../kernel";

export function ChoiceList({
  choice,
  ended,
  onSelect,
}: {
  choice: PendingChoice;
  ended: boolean;
  onSelect: (index: number) => void;
}) {
  return (
    <section className="choices" aria-label="待决选择">
      <p className="choice-prompt">? {choice.prompt}</p>
      <ul className="choice-list">
        {choice.options.map((option, i) => (
          <li key={option}>
            <button
              type="button"
              className="choice-button"
              disabled={ended}
              onClick={() => onSelect(i + 1)}
              aria-label={`选项 ${i + 1}：${option}`}
            >
              <span className="choice-number">{i + 1}</span>
              <span className="choice-text">{option}</span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}
