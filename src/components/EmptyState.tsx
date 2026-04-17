import { Terminal, UserPlus } from "lucide-react";

export function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="empty onboarding">
      <UserPlus size={32} strokeWidth={1} />
      <h2>Get started with Claudepot</h2>

      <div className="onboarding-steps">
        <div className="onboarding-step">
          <span className="onboarding-step-number">1</span>
          <div>
            <p className="onboarding-step-title">Sign into Claude Code</p>
            <p className="muted onboarding-step-detail">
              <Terminal size={12} />{" "}
              <code>claude auth login</code>
            </p>
          </div>
        </div>
        <div className="onboarding-step">
          <span className="onboarding-step-number">2</span>
          <div>
            <p className="onboarding-step-title">Import into Claudepot</p>
            <p className="muted onboarding-step-detail">
              Claudepot picks up the active CC credentials automatically.
            </p>
          </div>
        </div>
      </div>

      <button className="primary" onClick={onAdd}>
        Add current account
      </button>
      <p className="muted onboarding-repeat-hint">
        Repeat for each account you use.
      </p>
    </div>
  );
}
