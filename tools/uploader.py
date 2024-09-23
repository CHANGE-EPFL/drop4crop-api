#!/usr/bin/env python3

import requests
import getpass
import json
import os
import sys
import concurrent.futures
import logging
import typer
from keycloak import KeycloakOpenID
import sys

# Constants
APPLICATION_NAME: str = "Drop4Crop"
DROP4CROP_SERVER: str = "https://drop4crop-dev.epfl.ch"
TOKEN_CACHE_FILE: str = "token_cache.json"
UPLOAD_ENDPOINT: str = "/api/layers/uploads"
EXISTING_LAYERS_ENDPOINT: str = "/api/layers"
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

# Set up logger with timestamp
logger = logging.getLogger(__name__)

app = typer.Typer(
    no_args_is_help=True,
    add_completion=False,
    pretty_exceptions_show_locals=False,
)


def query_existing_layers(server, token):
    """Query the API to get existing layers and filenames."""
    headers = {"Authorization": f"Bearer {token}"}
    existing_files = set()

    try:
        response = requests.get(
            f"{server}{EXISTING_LAYERS_ENDPOINT}?filter=%7B%7D&range=%5B0%2C10000%5D&sort=%5B%22uploaded_at%22%2C%22DESC%22%5D",
            headers=headers,
        )

        if response.status_code == 200:
            data = response.json()

            # Iterate over the response and collect filenames
            for layer in data:
                filename = layer.get("filename")
                if filename:
                    existing_files.add(filename)

            logger.info(
                f"Found {len(existing_files)} existing layers on the server."
            )
        else:
            logger.error(
                f"Failed to query existing layers: {response.status_code} - {response.text}"
            )

    except Exception as e:
        logger.error(f"Error querying existing layers: {e}")

    return existing_files


def filter_existing_files(files_to_upload, existing_files, overwrite):
    """Filter out files that already exist on the server."""
    if overwrite:
        logger.info("Overwrite mode is enabled, all files will be uploaded.")
        return files_to_upload

    filtered_files = [
        (file_path, filename)
        for file_path, filename in files_to_upload
        if filename not in existing_files
    ]

    logger.info(
        f"Total files skipped (already exist): {len(files_to_upload) - len(filtered_files)}"
    )
    logger.info(f"Files planned for upload: {len(filtered_files)}")

    return filtered_files


def traverse_directory_and_build_filenames(base_directory: str):
    """Traverse directories and build the filenames according to the folder structure."""
    files_to_upload = []
    file_count = 0  # Initialize file count

    for root, dirs, files in os.walk(base_directory):
        for file in files:
            if file.endswith(".tif"):
                file_lower = file.lower().replace(".tif", "")
                relative_path = [
                    part.lower()
                    for part in os.path.relpath(root, base_directory).split(
                        os.sep
                    )
                ]

                # Crop-specific handling
                if (
                    len(relative_path) == 2
                    and relative_path[0].lower() == "crop specific parameters"
                ):
                    crop = file_lower.split("_")[0].lower()
                    if "areair" in file_lower:
                        variable = "mirca_area_irrigated"
                    elif "arearf" in file_lower:
                        variable = "mirca_area_rainfed"
                    elif "areatotal" in file_lower:
                        variable = "mirca_area_total"
                    elif "yield" in file_lower:
                        variable = "yield"
                    elif "production" in file_lower:
                        variable = "production"
                    else:
                        logger.warning(
                            f"Unknown crop-specific variable in file: {file}"
                        )
                        continue

                    if (
                        crop in CROP_ITEMS
                        and variable in CROP_SPECIFIC_VARIABLES
                    ):
                        filename = f"{crop}_{variable}.tif"
                        file_path = os.path.join(root, file)
                        files_to_upload.append((file_path, filename))
                    continue

                # General handling
                path_filtered = [
                    part
                    for part in relative_path
                    if part not in ["2005soc", "historical"]
                ]

                if len(path_filtered) < 5:
                    continue

                water_model = path_filtered[0]
                climate_model = path_filtered[1]
                scenario = path_filtered[2]
                year = path_filtered[3]
                crop = path_filtered[4]
                variable = file_lower

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
                    file_path = os.path.join(root, file)
                    files_to_upload.append((file_path, filename))

                # Update the file count and display progress on the same line
                file_count += 1
                sys.stdout.write(f"\rFiles found: {file_count}")
                sys.stdout.flush()

    # Ensure the cursor moves to the next line after the final count
    print(
        f"\nTotal valid files to upload (traverse mode): {len(files_to_upload)}"
    )
    return files_to_upload, len(files_to_upload)


def flattened_directory_build_filenames(flat_directory: str):
    """Process all .tif files in a flat directory."""
    files_to_upload = []

    for file in os.listdir(flat_directory):
        if file.endswith(".tif"):
            file_lower = file.lower().replace(
                ".tif", ""
            )  # Variable is the filename without extension
            file_parts = file_lower.split("_")

            if len(file_parts) == 6:  # General structure file
                crop, water_model, climate_model, scenario, variable, year = (
                    file_parts
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
                    files_to_upload.append(
                        (os.path.join(flat_directory, file), file)
                    )
            elif len(file_parts) == 2:  # Crop-specific structure
                crop, crop_specific_variable = file_parts
                if (
                    crop in CROP_ITEMS
                    and crop_specific_variable in CROP_SPECIFIC_VARIABLES
                ):
                    files_to_upload.append(
                        (os.path.join(flat_directory, file), file)
                    )
            else:
                logger.debug(f"Skipping invalid filename structure: {file}")

    logger.info(
        f"Total valid files to upload (flattened mode): {len(files_to_upload)}"
    )
    return files_to_upload


def load_token_cache(server: str):
    """Load the token cache for a specific server from a file."""
    if os.path.exists(TOKEN_CACHE_FILE):
        with open(TOKEN_CACHE_FILE, "r") as file:
            cache = json.load(file)
            return cache.get(
                server, None
            )  # Return the token cache for the specific server
    return None


def save_token_cache(server: str, token_data):
    """Save the token cache for a specific server to a file."""
    if os.path.exists(TOKEN_CACHE_FILE):
        with open(TOKEN_CACHE_FILE, "r") as file:
            cache = json.load(file)
    else:
        cache = {}

    cache[server] = token_data  # Store the token cache for the specific server

    with open(TOKEN_CACHE_FILE, "w") as file:
        json.dump(cache, file)


def get_token_from_cache(server: str, keycloak_openid):
    """Retrieve and refresh the token for a specific server from the cache if necessary."""
    token_data = load_token_cache(server)

    if token_data:
        try:
            new_token_data = keycloak_openid.refresh_token(
                token_data["refresh_token"]
            )
            save_token_cache(server, new_token_data)
            return new_token_data["access_token"]
        except Exception:
            logger.warning(
                f"Failed to refresh token for {server}, retrieving a new one."
            )
    return None


def get_new_token(server: str, keycloak_openid):
    """Get a new token using provided credentials for a specific server."""
    username = input(f"Enter your {APPLICATION_NAME} username for {server}: ")
    password = getpass.getpass(
        f"Enter your {APPLICATION_NAME} password for {server}: "
    )

    token_data = keycloak_openid.token(username, password)
    save_token_cache(server, token_data)

    return token_data["access_token"]


def get_token(server: str):
    """Authenticate with Keycloak for a specific server and obtain or refresh a token."""
    response = requests.get(f"{server}/api/config/keycloak")
    response.raise_for_status()
    keycloak_config = response.json()

    keycloak_openid = KeycloakOpenID(
        server_url=keycloak_config["url"],
        client_id=keycloak_config["clientId"],
        realm_name=keycloak_config["realm"],
        verify=True,
    )

    token = get_token_from_cache(server, keycloak_openid)
    if not token:
        token = get_new_token(server, keycloak_openid)

    return token


def upload_file(server, file_path, token, filename, overwrite_duplicates):
    """Upload a single file to the server."""
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
        logger.info(f"Successfully uploaded {filename}")
    else:
        logger.error(
            f"Failed to upload {filename}: {response.status_code} - {response.text}"
        )


def parallel_upload(
    files_to_upload, server, token, threads, overwrite_duplicates
):
    """Upload files in parallel."""
    with concurrent.futures.ThreadPoolExecutor(
        max_workers=threads
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
        for future in concurrent.futures.as_completed(futures):
            future.result()


@app.command()
def traverse(
    directory: str,
    server: str = DROP4CROP_SERVER,
    threads: int = DEFAULT_THREADS,
    overwrite: bool = False,
    noconfirm: bool = False,  # Added noconfirm flag for skipping confirmation
    debug: bool = False,
):
    """Traverse nested directories to find and upload files."""
    if debug:
        logging.basicConfig(
            level=logging.DEBUG,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )
    else:
        logging.basicConfig(
            level=logging.INFO,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )

    token = get_token(server)

    # Query existing layers
    existing_files = query_existing_layers(server, token)

    files_to_upload, num_files = traverse_directory_and_build_filenames(
        directory
    )

    # Filter out files that already exist on the server if overwrite is disabled
    files_to_upload = filter_existing_files(
        files_to_upload, existing_files, overwrite
    )

    logger.info(f"Server: {server}")
    logger.info(f"Files to upload: {len(files_to_upload)}")
    logger.info(f"Threads: {threads}")
    logger.info(f"Overwrite duplicates: {'Yes' if overwrite else 'No'}")

    # Proceed confirmation unless noconfirm is set
    if not noconfirm:
        proceed = input(
            "Do you want to proceed with these settings? [y/N]: "
        ).lower()
        if proceed != "y":
            logger.info("Operation cancelled.")
            raise typer.Exit()

    parallel_upload(files_to_upload, server, token, threads, overwrite)


@app.command()
def flattened(
    directory: str,
    server: str = DROP4CROP_SERVER,
    threads: int = DEFAULT_THREADS,
    overwrite: bool = False,
    noconfirm: bool = False,  # Added noconfirm flag for skipping confirmation
    debug: bool = False,
):
    """Upload files from a flat directory."""
    if debug:
        logging.basicConfig(
            level=logging.DEBUG,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )
    else:
        logging.basicConfig(
            level=logging.INFO,
            format="%(asctime)s - %(levelname)s - %(message)s",
        )

    logging.info("Getting authentication token")
    token = get_token(server)
    logging.info("Authenticated successfully")

    # Query existing layers
    logging.info("Querying existing layers")
    existing_files = query_existing_layers(server, token)
    files_to_upload = flattened_directory_build_filenames(directory)

    # Filter out files that already exist on the server if overwrite disabled
    files_to_upload = filter_existing_files(
        files_to_upload, existing_files, overwrite
    )

    logger.info(f"Server: {server}")
    logger.info(f"Files to upload: {len(files_to_upload)}")
    logger.info(f"Threads: {threads}")
    logger.info(f"Overwrite duplicates: {'Yes' if overwrite else 'No'}")

    # Proceed confirmation unless noconfirm is set
    if not noconfirm:
        proceed = input(
            "Do you want to proceed with these settings? [y/N]: "
        ).lower()
        if proceed != "y":
            logger.info("Operation cancelled.")
            raise typer.Exit()

    parallel_upload(files_to_upload, server, token, threads, overwrite)


if __name__ == "__main__":
    app()
