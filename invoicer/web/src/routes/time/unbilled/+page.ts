// /time/unbilled, To bill's old address (plan 46): a bookmark or an old link
// lands on /to-bill, its query kept. A `#token=` launch fragment is taken up
// first: the redirect drops the fragment, and the token is otherwise only
// read on the first request, after it.
import {redirect} from "@sveltejs/kit";
import {resolve} from "$app/paths";
import {adoptLaunchToken} from "#ui/api/client.js";
import type {PageLoad} from "./$types";

export const load: PageLoad = ({url}) => {
    adoptLaunchToken();
    redirect(308, `${resolve("/to-bill")}${url.search}`);
};
