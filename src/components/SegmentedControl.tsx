export function SegmentedControl<T extends string>({
  options,
  value,
  onChange,
}: {
  options: readonly { id: T; label: string }[];
  value: T;
  onChange: (id: T) => void;
}) {
  return (
    <div className="segmented-control" role="tablist">
      {options.map((opt) => (
        <button
          key={opt.id}
          role="tab"
          aria-selected={opt.id === value}
          className={`segmented-control-item ${opt.id === value ? "selected" : ""}`}
          onClick={() => onChange(opt.id)}
        >
          {opt.label}
        </button>
      ))}
    </div>
  );
}
