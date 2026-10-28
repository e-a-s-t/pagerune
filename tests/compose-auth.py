#!/usr/bin/env python3
"""Black-box authentication smoke test for a running local Compose stack."""

import http.cookiejar
import json
import subprocess
import urllib.error
import urllib.parse
import urllib.request
import uuid
from html.parser import HTMLParser

BASE = "http://localhost:8088"
KEYCLOAK = "http://keycloak.localhost:8081"


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


class LocalhostCookieProcessor(urllib.request.HTTPCookieProcessor):
    def http_request(self, request):
        request = super().http_request(request)
        if urllib.parse.urlsplit(request.full_url).hostname == "localhost":
            proxy_cookies = [cookie for cookie in self.cookiejar if cookie.name.startswith("_oauth2_proxy")]
            existing = request.get_header("Cookie", "")
            missing = [cookie for cookie in proxy_cookies if f"{cookie.name}=" not in existing]
            if missing:
                addition = "; ".join(f"{cookie.name}={cookie.value}" for cookie in missing)
                request.add_unredirected_header("Cookie", f"{existing}; {addition}" if existing else addition)
        return request


class FormParser(HTMLParser):
    def __init__(self):
        super().__init__()
        self.action = None
        self.values = {}
        self.in_form = False

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == "form" and self.action is None:
            self.action = attrs.get("action")
            self.in_form = True
        elif tag == "input" and self.in_form and attrs.get("name"):
            if attrs.get("type") == "hidden":
                self.values[attrs["name"]] = attrs.get("value", "")

    def handle_endtag(self, tag):
        if tag == "form":
            self.in_form = False


def open_response(opener, url, data=None, headers=None):
    request = urllib.request.Request(url, data=data, headers=headers or {})
    try:
        return opener.open(request, timeout=15)
    except urllib.error.HTTPError as response:
        return response


def assert_status(opener, path, expected, headers=None):
    response = open_response(opener, BASE + path, headers=headers)
    assert response.status == expected, f"{path}: expected {expected}, got {response.status}"
    return response


def login_opener(username, password):
    jar = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), LocalhostCookieProcessor(jar))
    response = opener.open(BASE + "/management", timeout=20)
    body = response.read().decode()
    assert response.geturl().startswith(KEYCLOAK), f"login did not reach Keycloak: {response.geturl()}"
    form = FormParser()
    form.feed(body)
    assert form.action, "Keycloak login form was not found"
    values = dict(form.values)
    values.update({"username": username, "password": password})
    login_url = urllib.parse.urljoin(response.geturl(), form.action)
    keycloak_cookies = "; ".join(
        f"{cookie.name}={cookie.value}"
        for cookie in jar
        if cookie.domain.lstrip(".") == "keycloak.localhost"
    )
    result = open_response(
        opener,
        login_url,
        data=urllib.parse.urlencode(values).encode(),
        headers={"Content-Type": "application/x-www-form-urlencoded", "Cookie": keycloak_cookies},
    )
    return opener, result


def main():
    discovery = urllib.request.urlopen(
        KEYCLOAK + "/realms/pagerune/.well-known/openid-configuration", timeout=15
    )
    issuer = json.loads(discovery.read().decode())["issuer"]
    assert issuer == KEYCLOAK + "/realms/pagerune", f"unexpected OIDC issuer: {issuer}"

    public = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    response = assert_status(public, "/", 200)
    assert "PageRune" in response.read().decode()
    assert_status(public, "/manage", 200)
    assert_status(public, f"/subscribe/{uuid.uuid4()}", 404)
    assert_status(public, "/verify/invalid-token", 404)
    assert_status(public, "/unsubscribe/invalid-token", 404)
    assert_status(public, "/static/pagerune.css", 200)

    management = open_response(public, BASE + "/management")
    assert management.status == 302
    assert "keycloak.localhost" in management.headers.get("Location", "")
    spoofed_headers = {
        "X-Forwarded-User": "admin",
        "X-Forwarded-Groups": "/pagerune-management",
        "X-Auth-Request-User": "admin",
        "X-Auth-Request-Groups": "/pagerune-management",
    }
    spoofed_management = open_response(public, BASE + "/management", headers=spoofed_headers)
    assert spoofed_management.status == 302, "spoofed identity headers granted management access"
    assert_status(public, "/api/v1/sites", 401, spoofed_headers)
    api = assert_status(public, "/api/v1/sites", 401, {"Accept": "application/json"})
    assert not api.headers.get("Location", "").startswith(KEYCLOAK)

    # Compose must not publish the backend listener independently of OAuth2 Proxy.
    direct = subprocess.run(
        ["docker", "inspect", "--format", "{{json .HostConfig.PortBindings}}", "pagerune-pagerune-1"],
        capture_output=True,
        text=True,
        check=False,
    )
    assert direct.returncode == 0, "could not inspect the PageRune container port bindings"
    bindings = __import__("json").loads(direct.stdout)
    assert not bindings.get("8080/tcp"), "PageRune backend port is published"

    invalid, invalid_result = login_opener("admin", "not-the-password")
    assert invalid_result.geturl().startswith(KEYCLOAK), "invalid credentials reached management"
    assert_status(invalid, "/oauth2/auth", 401)

    authenticated, result = login_opener("admin", "admin")
    assert result.geturl().startswith(BASE + "/management"), "admin login did not reach management"
    assert "Registered websites" in result.read().decode()
    assert_status(authenticated, "/oauth2/auth", 202)

    authenticated.open(BASE + "/oauth2/sign_out?rd=%2F", timeout=15).read()
    assert_status(authenticated, "/oauth2/auth", 401)
    print("Compose authentication smoke tests passed")


if __name__ == "__main__":
    main()
