interface Props {
  size?: number;
  className?: string;
  ariaLabel?: string;
}

export function Logo({ size = 32, className, ariaLabel }: Props) {
  return (
    <img
      src="/claudepot-logo.svg"
      width={size}
      height={size}
      alt={ariaLabel ?? ""}
      role={ariaLabel ? "img" : "presentation"}
      aria-hidden={ariaLabel ? undefined : true}
      className={className}
    />
  );
}
