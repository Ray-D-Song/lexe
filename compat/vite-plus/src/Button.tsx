import type { ButtonHTMLAttributes } from "react";
import "./style.css";

export function Button(props: ButtonHTMLAttributes<HTMLButtonElement>) {
  return <button className="lexe-button" {...props} />;
}
