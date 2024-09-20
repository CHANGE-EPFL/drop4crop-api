#!/usr/bin/env python3

import requests
import getpass
import json
import os
import concurrent.futures
import argparse
from keycloak import KeycloakOpenID

# Constants
APPLICATION_NAME: str = "Drop4Crop"
DROP4CROP_SERVER: str = "https://drop4crop-dev.epfl.ch"
TOKEN_CACHE_FILE: str = "token_cache.json"
UPLOAD_ENDPOINT: str = "/api/layers/uploads"
DEFAULT_THREADS: int = 10
OVERWRITE_DUPLICATES: bool = True


def load_token_cache():
    """Load the token cache from a file."""
    if os.path.exists(TOKEN_CACHE_FILE):
        with open(TOKEN_CACHE_FILE, "r") as file:
            return json.load(file)
    return None


def save_token_cache(token_data):
    """Save the token cache to a file."""
    with open(TOKEN_CACHE_FILE, "w") as file:
        json.dump(token_data, file)


def get_token_from_cache(keycloak_openid):
    """Retrieve and refresh the token from the cache if necessary."""
    token_data = load_token_cache()

    if token_data:
        try:
            # Refresh the token
            new_token_data = keycloak_openid.refresh_token(
                token_data["refresh_token"]
            )
            save_token_cache(new_token_data)
            return new_token_data["access_token"]
        except Exception:
            print("Failed to refresh token, retrieving a new one.")

    return None


def get_new_token(keycloak_openid):
    """Get a new token using provided credentials."""
    username = input(f"Enter your {APPLICATION_NAME} username: ")
    password = getpass.getpass(f"Enter your {APPLICATION_NAME} password: ")

    # Retrieve the token
    token_data = keycloak_openid.token(username, password)
    save_token_cache(token_data)

    return token_data["access_token"]


def get_token(server: str):
    """Get the keycloak config from /api/config/keycloak and use it to
    authenticate with Keycloak and obtain or refresh a token.
    """
    response = requests.get(f"{server}/api/config/keycloak")
    response.raise_for_status()
    keycloak_config = response.json()

    keycloak_openid = KeycloakOpenID(
        server_url=keycloak_config["url"],
        client_id=keycloak_config["clientId"],
        realm_name=keycloak_config["realm"],
        verify=True,  # Optional, depending on your Keycloak setup
    )

    # Try to get token from cache
    token = get_token_from_cache(keycloak_openid)

    if not token:
        # If no valid token is found in the cache, get a new one
        token = get_new_token(keycloak_openid)

    return token


def upload_file(server, file_path, token, overwrite_duplicates):
    """Upload a single file to the server.

    If the server responds with any server error, retry the file upload.
    """
    with open(file_path, "rb") as f:
        files = {"file": (os.path.basename(file_path), f)}
        headers = {"Authorization": f"Bearer {token}"}

        response = requests.post(
            f"{server}{UPLOAD_ENDPOINT}",
            files=files,
            headers=headers,
            params={"overwrite_duplicates": overwrite_duplicates},
        )

    if response.status_code == 200:
        print(f"Successfully uploaded {file_path}")
    elif response.status_code >= 500:
        print(
            f"Failed to upload {file_path}: {response.status_code}, "
            f"retrying..."
        )
        upload_file(
            server=server,
            file_path=file_path,
            token=token,
            overwrite_duplicates=overwrite_duplicates,
        )
    elif response.status_code == 409 and not overwrite_duplicates:
        print(f"File already exists on the server, skipping: {file_path}")
    else:
        print(
            f"Failed to upload {file_path}: {response.status_code}, "
            f"{response.text}"
        )


def parallel_upload(
    server, directory, token, num_threads, overwrite_duplicates
):
    """Upload all files in a directory in parallel using a specified number
    of threads.

    The files are only uploaded if they are .tif files.
    """
    files_to_upload = [
        os.path.join(directory, file)
        for file in os.listdir(directory)
        if file.endswith(".tif")
    ]

    with concurrent.futures.ThreadPoolExecutor(
        max_workers=num_threads
    ) as executor:
        futures = [
            executor.submit(
                upload_file, server, file_path, token, overwrite_duplicates
            )
            for file_path in files_to_upload
        ]

        # Wait for all futures to complete
        for future in concurrent.futures.as_completed(futures):
            future.result()


if __name__ == "__main__":

    # Use argparse to get all the input parameters
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--server",
        help="The server URL to upload to",
        default=DROP4CROP_SERVER,
    )
    parser.add_argument(
        "--directory",
        help="The directory containing the files to upload",
        default=os.getcwd(),
    )
    parser.add_argument(
        "--threads",
        type=int,
        help=f"The number of threads to use for uploading [default: {DEFAULT_THREADS}]",
        default=DEFAULT_THREADS,
    )
    parser.add_argument(
        "--overwrite",
        action="store_true",
        help="Overwrite duplicate files if they exist on the server",
    )
    parser.add_argument(
        "--noconfirm",
        action="store_true",
        help="Skip the confirmation prompt before uploading files",
    )
    args = parser.parse_args()

    server = args.server
    directory = os.path.normpath(args.directory)
    num_threads = args.threads
    overwrite_duplicates = args.overwrite

    # Count .tif files in the directory
    files_to_upload = [
        file for file in os.listdir(directory) if file.endswith(".tif")
    ]
    num_files = len(files_to_upload)

    # Get the token, using cache or prompting for username and password if needed
    token = get_token(server)
    print(f"Contacting server at {server}...")

    # Print if token is valid
    if not token:
        print("Failed to obtain a valid authentication token. Exiting...")
        exit(1)
    else:
        print("Authentication token is valid. Continuing...")
    print()

    # Summary before execution
    print(f"Server: {server}")
    print(f"Directory: {directory}")
    print(f"Number of .tif files to upload: {num_files}")
    print(f"Threads: {num_threads}")
    print(f"Overwrite duplicates: {'Yes' if overwrite_duplicates else 'No'}")
    print()

    # Ask for confirmation unless --noconfirm is specified
    if not args.noconfirm:
        if (
            input(
                "Do you want to proceed with these settings? [y/N]: "
            ).lower()
            != "y"
        ):
            print("Operation cancelled.")
            exit(0)

    # Perform the upload in parallel
    parallel_upload(
        server, directory, token, num_threads, overwrite_duplicates
    )
