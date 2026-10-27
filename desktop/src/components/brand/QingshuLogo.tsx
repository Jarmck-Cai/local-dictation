/** Optical sizes match src-tauri/icons/qingshu.svg, including the 2px caps. */
export const QingshuLogo = ({
  size = 32,
  className,
  tile = true,
}: {
  size?: number;
  className?: string;
  tile?: boolean;
}) => {
  const base = size <= 20 ? 16 : size <= 24 ? 24 : 32;
  const geometry =
    base === 16
      ? [
          [3, 5, 2, 6, 1],
          [7, 2, 2, 12, 1],
          [11, 5, 2, 6, 0],
          [10, 4, 4, 2, 0],
          [10, 10, 4, 2, 0],
        ]
      : base === 24
        ? [
            [5, 8, 3, 9, 1.5],
            [11, 3, 3, 18, 1.5],
            [18, 7, 2, 10, 0],
            [16, 6, 6, 2, 0],
            [16, 16, 6, 2, 0],
          ]
        : [
            [6, 10, 4, 12, 2],
            [14, 4, 4, 24, 2],
            [23, 9, 2, 14, 0],
            [20, 8, 8, 2, 0],
            [20, 22, 8, 2, 0],
          ];
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${base} ${base}`}
      fill="none"
      className={className}
      aria-hidden="true"
    >
      {tile && <rect width={base} height={base} rx={base / 4} fill="#a9d6b9" />}
      <g fill={tile ? "#28543b" : "currentColor"}>
        {geometry.map(([x, y, width, height, rx], index) => (
          <rect key={index} x={x} y={y} width={width} height={height} rx={rx} />
        ))}
      </g>
    </svg>
  );
};
