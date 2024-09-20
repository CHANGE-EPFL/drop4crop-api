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


# The various items used to match in the directory structure
CROP_ITEMS = [
    "barley",
    "maize",
    "potato",
    "rice",
    "sorghum",
    "soy",
    "sugarcane",
    "wheat",
]
CROP_SPECIFIC_VARIABLES = [
    "mirca_area_irrigated",
    "mirca_area_total",
    "mirca_rainfed",
    "yield",
    "production",
]
GLOBAL_WATER_MODELS = [
    "cwatm",
    "h08",
    "lpjml",
    "matsiro",
    "pcr-globwb",
    "watergap2",
]
CLIMATE_MODELS = ["gfdl-esm2m", "hadgem2-es", "ipsl-cm5a-lr", "miroc5"]
SCENARIOS = ["rcp26", "rcp60", "rcp85"]
VARIABLES = [
    "vwc",
    "vwcb",
    "vwcg",
    "vwcg_perc",
    "vwcb_perc",
    "wf",
    "wfb",
    "wfg",
    "etb",
    "etg",
    "rb",
    "rg",
    "wdb",
    "wdg",
]


def traverse_directory_and_build_filenames(base_directory):
    """Traverse directories and build the filenames according to the folder structure."""
    files_to_upload = []

    for root, dirs, files in os.walk(base_directory):
        for file in files:
            if file.endswith(".tif"):
                # Lowercase the filename for consistency
                file_lower = file.lower().replace(
                    ".tif", ""
                )  # Variable is the filename without extension

                # Print each found .tif file
                print(f"Found file: {file} in {root}")

                # Split the path and start identifying the elements, ensuring everything is lowercase
                relative_path = [
                    part.lower()
                    for part in os.path.relpath(root, base_directory).split(
                        os.sep
                    )
                ]
                print(f"Relative path parts: {relative_path}")

                # If the structure contains "crop specific parameters"
                if (
                    len(relative_path) >= 2
                    and relative_path[0] == "crop specific parameters"
                ):
                    crop = relative_path[-1]
                    crop_specific_variable = relative_path[1]
                    print(
                        f"Processing as crop-specific: crop={crop}, crop_specific_variable={crop_specific_variable}"
                    )

                    if (
                        crop in CROP_ITEMS
                        and crop_specific_variable in CROP_SPECIFIC_VARIABLES
                    ):
                        filename = f"{crop}_{crop_specific_variable}.tif"
                        print(f"Valid crop-specific filename: {filename}")
                    else:
                        print(
                            f"Skipping invalid crop-specific structure: crop={crop}, variable={crop_specific_variable}"
                        )
                        print(
                            f"Expected crops: {CROP_ITEMS}, Expected variables: {CROP_SPECIFIC_VARIABLES}"
                        )
                        continue  # Skip if the structure is incorrect
                else:
                    # For general structure, check if "2005soc" or "historical" is part of the path and ignore them
                    path_filtered = [
                        part
                        for part in relative_path
                        if part not in ["2005soc", "historical"]
                    ]

                    if len(path_filtered) < 5:
                        print(
                            f"Skipping incomplete structure: {path_filtered}"
                        )
                        continue  # Skip if the directory structure is not complete

                    water_model = path_filtered[0]
                    climate_model = path_filtered[1]
                    scenario = path_filtered[2]
                    year = path_filtered[
                        3
                    ]  # The year is now in the fourth position
                    crop = path_filtered[
                        4
                    ]  # The crop is now in the fifth position

                    # Use the filename as the variable
                    variable = file_lower  # Extract variable from the file name (already lowercased)
                    print(
                        f"Processing general structure: crop={crop}, water_model={water_model}, climate_model={climate_model}, scenario={scenario}, variable={variable}, year={year}"
                    )

                    if all(
                        [
                            crop in CROP_ITEMS,
                            water_model in GLOBAL_WATER_MODELS,
                            climate_model in CLIMATE_MODELS,
                            scenario in SCENARIOS,
                            variable in VARIABLES,
                        ]
                    ):
                        filename = f"{crop}_{water_model}_{climate_model}_{scenario}_{variable}_{year}.tif"
                        print(f"Valid general filename: {filename}")
                    else:
                        print(
                            f"Skipping invalid structure: crop={crop}, water_model={water_model}, climate_model={climate_model}, scenario={scenario}, variable={variable}"
                        )
                        print(f"Expected crops: {CROP_ITEMS}")
                        print(f"Expected water models: {GLOBAL_WATER_MODELS}")
                        print(f"Expected climate models: {CLIMATE_MODELS}")
                        print(f"Expected scenarios: {SCENARIOS}")
                        print(f"Expected variables: {VARIABLES}")
                        continue  # Skip if the structure doesn't match the expected values

                file_path = os.path.join(root, file)
                files_to_upload.append((file_path, filename))

    print(f"Total valid files to upload: {len(files_to_upload)}")
    # Return the files to upload and their count
    return files_to_upload, len(files_to_upload)


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


def upload_file(server, file_path, token, filename, overwrite_duplicates):
    """Upload a single file to the server with the appropriate filename."""
    with open(file_path, "rb") as f:
        files = {"file": (filename, f)}
        headers = {"Authorization": f"Bearer {token}"}

        response = requests.post(
            f"{server}{UPLOAD_ENDPOINT}",
            files=files,
            headers=headers,
            params={"overwrite_duplicates": overwrite_duplicates},
        )

    if response.status_code == 200:
        print(f"Successfully uploaded {filename}")
    else:
        print(
            f"Failed to upload {filename}: {response.status_code} - {response.text}"
        )


def parallel_upload(
    files_to_upload,
    server,
    directory,
    token,
    num_threads,
    overwrite_duplicates,
):
    """Upload all files from the directory in parallel."""

    with concurrent.futures.ThreadPoolExecutor(
        max_workers=num_threads
    ) as executor:
        futures = [
            executor.submit(
                upload_file,
                server,
                file_path,
                token,
                filename,
                overwrite_duplicates,
            )
            for file_path, filename in files_to_upload
        ]

        # Wait for all uploads to complete
        for future in concurrent.futures.as_completed(futures):
            future.result()


if __name__ == "__main__":
    # Use argparse to get input parameters
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

    # Get token
    token = get_token(server)

    # Do this again, beforehand to be able ot see how many files
    files_to_upload, num_files = traverse_directory_and_build_filenames(
        directory
    )

    # Summary
    print(f"Server: {server}")
    print(f"Directory: {directory}")
    print(f"Files to upload: {num_files}")
    print(f"Threads: {num_threads}")
    print(f"Overwrite duplicates: {'Yes' if overwrite_duplicates else 'No'}")

    if not args.noconfirm:
        confirm = input("Do you want to proceed with these settings? [y/N]: ")
        if confirm.lower() != "y":
            print("Operation cancelled.")
            exit(0)

    # Perform the parallel upload
    parallel_upload(
        files_to_upload,
        server,
        directory,
        token,
        num_threads,
        overwrite_duplicates,
    )
