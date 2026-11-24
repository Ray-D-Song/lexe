/// <reference path="../buffer.d.ts" />
/// <reference path="../url.d.ts" />
import { Url, fileURLToPathBuffer, format, parse, resolve, resolveObject } from "url";

const parsed: Url = parse("http://a.com/b?c=d");
const withQuery: Url = parse("/p?x=1", true, true);
const resolved: string = resolve("http://a.com/b", "../c");
const resolvedObject: Url = resolveObject("http://a.com/b", parse("../c"));
const formatted: string = format({ protocol: "http", host: "a.com", pathname: "/p", query: { a: "1" } });
const fromString: string = format("http://a.com/x y");
const fromUrl: string = format(new URL("http://a.com/"), { fragment: false });
const bytes = fileURLToPathBuffer(new URL("file:///a"), { windows: false });
const isUrl: boolean = parsed instanceof Url && new Url().parse("/x") instanceof Url;

export { bytes, formatted, fromString, fromUrl, isUrl, resolved, resolvedObject, withQuery };
