export function EmptyState({ onAdd }: { onAdd: () => void }) {
  return (
    <div className="empty">
      <h2>No accounts yet</h2>
      <p className="muted">
        Add your first account — Claudepot will pick up whichever one Claude
        Code is currently signed into.
      </p>
      <button className="primary" onClick={onAdd}>
        + Add account
      </button>
    </div>
  );
}
